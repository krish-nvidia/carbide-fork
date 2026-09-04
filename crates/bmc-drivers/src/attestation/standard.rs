/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish ComponentIntegrity attestation driver.
//!
//! `nv-redfish` has no ComponentIntegrity model, so the resources are read
//! through the minimal typed views below.

use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{
    Attestation, AttestationEvidence, CaCertificate, ComponentIntegritySummary, DriverOutcome,
    Fetched, OpCx, OperationReference, PlatformError,
};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, EntityTypeRef, ModificationResponse, NavProperty, ODataETag, ODataId};
use nv_redfish::schema::software_inventory::SoftwareInventory;
use serde::Deserialize;
use serde_json::json;

/// Standard ComponentIntegrity driver.
pub(crate) struct StandardComponentIntegrity {
    /// Maps a component id to its firmware-inventory id; `None` when the
    /// platform defines no such relationship.
    pub(crate) firmware_inventory: Option<fn(&str) -> Option<String>>,
}

pub(crate) static STANDARD_ATTESTATION: StandardComponentIntegrity = StandardComponentIntegrity {
    firmware_inventory: None,
};

/// Resource id of the standard ComponentIntegrity collection, which the
/// service root does not link.
fn component_collection_id() -> ODataId {
    ODataId::from("/redfish/v1/ComponentIntegrity".to_string())
}

#[derive(Debug, Deserialize)]
struct ComponentCollection {
    #[serde(rename = "@odata.id", default = "component_collection_id")]
    odata_id: ODataId,
    #[serde(rename = "Members")]
    members: Vec<NavProperty<Component>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Component {
    #[serde(rename = "@odata.id")]
    odata_id: ODataId,
    id: String,
    name: String,
    component_integrity_enabled: bool,
    component_integrity_type: String,
    component_integrity_type_version: String,
    #[serde(rename = "SPDM")]
    spdm: Option<Spdm>,
    actions: Option<ComponentActions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Spdm {
    identity_authentication: IdentityAuthentication,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IdentityAuthentication {
    responder_authentication: ResponderAuthentication,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ResponderAuthentication {
    component_certificate: CertificateReference,
}

#[derive(Debug, Deserialize)]
struct CertificateReference {
    #[serde(rename = "@odata.id")]
    odata_id: ODataId,
}

#[derive(Debug, Deserialize)]
struct ComponentActions {
    #[serde(rename = "#ComponentIntegrity.SPDMGetSignedMeasurements")]
    get_signed_measurements: Option<SignedMeasurementsAction>,
}

#[derive(Debug, Deserialize)]
struct SignedMeasurementsAction {
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Certificate {
    #[serde(rename = "@odata.id")]
    odata_id: ODataId,
    certificate_string: String,
    certificate_type: String,
    certificate_usage_types: Vec<String>,
    id: String,
    name: String,
    #[serde(rename = "SPDM")]
    spdm: CertificateSlot,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CertificateSlot {
    slot_id: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Evidence {
    hashing_algorithm: String,
    signed_measurements: String,
    signing_algorithm: String,
    version: String,
}

/// A task some BMCs return in the body of a signed-measurements request.
#[derive(Debug, Deserialize)]
struct EvidenceTask {
    #[serde(rename = "@odata.id")]
    odata_id: Option<ODataId>,
}

impl EntityTypeRef for ComponentCollection {
    fn odata_id(&self) -> &ODataId {
        &self.odata_id
    }

    fn etag(&self) -> Option<&ODataETag> {
        None
    }
}

impl EntityTypeRef for Component {
    fn odata_id(&self) -> &ODataId {
        &self.odata_id
    }

    fn etag(&self) -> Option<&ODataETag> {
        None
    }
}

impl EntityTypeRef for Certificate {
    fn odata_id(&self) -> &ODataId {
        &self.odata_id
    }

    fn etag(&self) -> Option<&ODataETag> {
        None
    }
}

async fn components<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Vec<Arc<Component>>, PlatformError> {
    let collection = cx
        .bmc()
        .get::<ComponentCollection>(&component_collection_id())
        .await
        .map_err(|error| cx.map_bmc_error(error))?;
    let mut components = Vec::with_capacity(collection.members.len());
    for member in &collection.members {
        components.push(
            member
                .get(cx.bmc())
                .await
                .map_err(|error| cx.map_bmc_error(error))?,
        );
    }
    Ok(components)
}

async fn component<B: Bmc>(
    cx: &OpCx<'_, B>,
    component_id: &str,
) -> Result<Arc<Component>, PlatformError> {
    components(cx)
        .await?
        .into_iter()
        .find(|component| component.id == component_id)
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("ComponentIntegrity resource {component_id} was not found"),
        })
}

fn evidence_target(component: &Component) -> Result<&str, PlatformError> {
    component
        .actions
        .as_ref()
        .and_then(|actions| actions.get_signed_measurements.as_ref())
        .map(|action| action.target.as_str())
        .ok_or(PlatformError::Unsupported)
}

#[async_trait]
impl<B: Bmc> Attestation<B> for StandardComponentIntegrity {
    async fn components(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<ComponentIntegritySummary>, PlatformError> {
        Ok(components(cx)
            .await?
            .into_iter()
            .map(|component| ComponentIntegritySummary {
                id: component.id.clone(),
                name: component.name.clone(),
                enabled: component.component_integrity_enabled,
                component_type: component.component_integrity_type.clone(),
                component_type_version: component.component_integrity_type_version.clone(),
            })
            .collect())
    }

    async fn firmware_for_component(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<Arc<SoftwareInventory>, PlatformError> {
        let inventory_id = self
            .firmware_inventory
            .and_then(|map| map(component_id))
            .ok_or(PlatformError::Unsupported)?;
        cx.service_root()
            .update_service()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .firmware_inventories()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .into_iter()
            .find(|inventory| inventory.id().into_inner() == inventory_id)
            .map(|inventory| inventory.raw())
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: format!("firmware inventory {inventory_id} was not found"),
            })
    }

    async fn ca_certificate(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<CaCertificate, PlatformError> {
        let component = component(cx, component_id).await?;
        let certificate_id = component
            .spdm
            .as_ref()
            .map(|spdm| {
                &spdm
                    .identity_authentication
                    .responder_authentication
                    .component_certificate
                    .odata_id
            })
            .ok_or(PlatformError::Unsupported)?;
        let certificate = cx
            .bmc()
            .get::<Certificate>(certificate_id)
            .await
            .map_err(|error| cx.map_bmc_error(error))?;
        Ok(CaCertificate {
            certificate_string: certificate.certificate_string.clone(),
            certificate_type: certificate.certificate_type.clone(),
            certificate_usage_types: certificate.certificate_usage_types.clone(),
            id: certificate.id.clone(),
            name: certificate.name.clone(),
            slot_id: certificate.spdm.slot_id,
        })
    }

    async fn trigger_evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
        nonce: &[u8],
    ) -> Result<DriverOutcome, PlatformError> {
        let component = component(cx, component_id).await?;
        let target = ODataId::from(evidence_target(&component)?.to_string());
        let response = cx
            .bmc()
            .create::<_, EvidenceTask>(&target, &json!({"Nonce": hex::encode(nonce)}))
            .await
            .map_err(|error| cx.map_bmc_error(error))?;
        match response {
            ModificationResponse::Entity(EvidenceTask {
                odata_id: Some(uri),
            }) => Ok(DriverOutcome::accepted(OperationReference::RedfishTask {
                uri,
                retry_after_seconds: None,
            })),
            ModificationResponse::Entity(EvidenceTask { odata_id: None }) => {
                Err(PlatformError::InvalidResponse {
                    message: "evidence task response has no @odata.id".to_string(),
                })
            }
            other => Ok(DriverOutcome::from(other)),
        }
    }

    async fn evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<AttestationEvidence, PlatformError> {
        let component = component(cx, component_id).await?;
        let target = ODataId::from(format!("{}/data", evidence_target(&component)?));
        let evidence = cx
            .bmc()
            .get::<Fetched<Evidence>>(&target)
            .await
            .map_err(|error| cx.map_bmc_error(error))?;
        Ok(AttestationEvidence {
            hashing_algorithm: evidence.hashing_algorithm.clone(),
            signed_measurements: evidence.signed_measurements.clone(),
            signing_algorithm: evidence.signing_algorithm.clone(),
            version: evidence.version.clone(),
        })
    }
}
