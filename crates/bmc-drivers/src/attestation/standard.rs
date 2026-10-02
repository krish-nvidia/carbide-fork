/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish ComponentIntegrity attestation driver.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bmc_platform::{
    Attestation, AttestationEvidence, CaCertificate, ComponentIntegritySummary, EvidenceProgress,
    OpCx, PendingEvidence, PlatformError,
};
use nv_redfish::component_integrity::{ComponentIntegrity, SpdmGetSignedMeasurementsResponse};
use nv_redfish::core::{AsyncTask, Bmc};
use nv_redfish::schema::software_inventory::SoftwareInventory;
use nv_redfish::task_service::AsyncActionResult;
use serde::Serialize;

/// Standard ComponentIntegrity attestation.
pub(crate) struct StandardAttestation;

#[async_trait]
impl<B: Bmc> Attestation<B> for StandardAttestation {
    fn standard(&self) -> &dyn Attestation<B> {
        self
    }

    async fn components(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<ComponentIntegritySummary>, PlatformError> {
        components(cx)
            .await?
            .iter()
            .map(|component| {
                let raw = component.raw();
                Ok(ComponentIntegritySummary {
                    id: raw.id.clone(),
                    name: raw.name.clone(),
                    enabled: raw
                        .component_integrity_enabled
                        .ok_or_else(|| missing(&raw.id, "ComponentIntegrityEnabled"))?,
                    component_type: wire_name(&raw.component_integrity_type)?,
                    component_type_version: raw.component_integrity_type_version.clone(),
                })
            })
            .collect()
    }

    /// Redfish defines no relationship between a component and its firmware
    /// inventory; platforms that have one override this.
    async fn firmware_for_component(
        &self,
        _cx: &OpCx<'_, B>,
        _component_id: &str,
    ) -> Result<Arc<SoftwareInventory>, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn ca_certificate(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<CaCertificate, PlatformError> {
        let certificate = component(cx, component_id)
            .await?
            .component_certificate()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .raw();
        let slot_id = certificate
            .spdm
            .as_ref()
            .and_then(|spdm| spdm.slot_id.flatten())
            .ok_or_else(|| missing(&certificate.id, "SPDM.SlotId"))?;
        Ok(CaCertificate {
            certificate_string: certificate
                .certificate_string
                .clone()
                .flatten()
                .ok_or_else(|| missing(&certificate.id, "CertificateString"))?,
            certificate_type: wire_name(
                &certificate
                    .certificate_type
                    .flatten()
                    .ok_or_else(|| missing(&certificate.id, "CertificateType"))?,
            )?,
            certificate_usage_types: certificate
                .certificate_usage_types
                .clone()
                .flatten()
                .ok_or_else(|| missing(&certificate.id, "CertificateUsageTypes"))?
                .iter()
                .map(wire_name)
                .collect::<Result<_, _>>()?,
            id: certificate.id.clone(),
            name: certificate.name.clone(),
            slot_id: u16::try_from(slot_id).map_err(|_| PlatformError::InvalidResponse {
                message: format!(
                    "certificate {} reports SPDM slot {slot_id} out of range",
                    certificate.id
                ),
            })?,
        })
    }

    /// Requests signed measurements, returning them when the BMC answers at
    /// once and otherwise the operation to poll.
    async fn request_evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
        nonce: &[u8],
    ) -> Result<EvidenceProgress, PlatformError> {
        let result = component(cx, component_id)
            .await?
            .spdm_get_signed_measurements(Some(hex::encode(nonce)), None, None)
            .await
            .map_err(|error| cx.map_redfish_error(error))?;
        match result.pending_task() {
            Some(task) => Ok(EvidenceProgress::Pending(pending(task))),
            None => advance(cx, result).await,
        }
    }

    async fn poll_evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
        pending: &PendingEvidence,
    ) -> Result<EvidenceProgress, PlatformError> {
        let component = component(cx, component_id).await?;
        let raw = component.raw();
        let action = raw
            .actions
            .as_ref()
            .and_then(|actions| actions.spdm_get_signed_measurements.as_ref())
            .ok_or(PlatformError::Unsupported)?;
        let task = AsyncTask {
            location: pending.uri.clone().into(),
            retry_after: pending.retry_after_seconds.map(Duration::from_secs),
        };
        advance(cx, AsyncActionResult::resume(action, task)).await
    }
}

async fn components<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Vec<ComponentIntegrity<B>>, PlatformError> {
    cx.service_root()
        .component_integrity()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

async fn component<B: Bmc>(
    cx: &OpCx<'_, B>,
    component_id: &str,
) -> Result<ComponentIntegrity<B>, PlatformError> {
    components(cx)
        .await?
        .into_iter()
        .find(|component| component.raw().id == component_id)
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("ComponentIntegrity resource {component_id} was not found"),
        })
}

/// Runs one polling step.
async fn advance<B: Bmc>(
    cx: &OpCx<'_, B>,
    mut result: AsyncActionResult<SpdmGetSignedMeasurementsResponse>,
) -> Result<EvidenceProgress, PlatformError> {
    match result
        .poll_result(cx.bmc())
        .await
        .map_err(|error| cx.map_redfish_error(error))?
    {
        Some(measurements) => Ok(EvidenceProgress::Ready(AttestationEvidence {
            hashing_algorithm: measurements.hashing_algorithm,
            signed_measurements: measurements.signed_measurements,
            signing_algorithm: measurements.signing_algorithm,
            version: measurements.version,
        })),
        None => result
            .pending_task()
            .map(|task| EvidenceProgress::Pending(pending(task)))
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: "signed-measurements request reported neither a result nor a task"
                    .to_string(),
            }),
    }
}

fn pending(task: &AsyncTask) -> PendingEvidence {
    PendingEvidence {
        uri: task.location.0.clone(),
        retry_after_seconds: task.retry_after.map(|delay| delay.as_secs()),
    }
}

/// The Redfish spelling of a schema enum value, such as `SPDM` or `PEM`.
fn wire_name(value: &impl Serialize) -> Result<String, PlatformError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        _ => Err(PlatformError::InvalidResponse {
            message: "schema enum value has no string form".to_string(),
        }),
    }
}

fn missing(resource: &str, property: &str) -> PlatformError {
    PlatformError::InvalidResponse {
        message: format!("{resource} does not report {property}"),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::*;
    use crate::test_support::{Fixture, body};

    const COLLECTION: &str = "/redfish/v1/ComponentIntegrity";
    const COMPONENT: &str = "/redfish/v1/ComponentIntegrity/HGX_IRoT_GPU_0";
    const TARGET: &str = "/redfish/v1/ComponentIntegrity/HGX_IRoT_GPU_0/Actions/ComponentIntegrity.SPDMGetSignedMeasurements";
    const TASK: &str = "/redfish/v1/TaskService/Tasks/95";
    const DATA: &str = "/redfish/v1/ComponentIntegrity/HGX_IRoT_GPU_0/Actions/ComponentIntegrity.SPDMGetSignedMeasurements/data";

    #[tokio::test]
    async fn evidence_request_persists_the_task_and_a_poll_reads_the_recorded_result() {
        let task = |state: &str, headers: serde_json::Value| {
            json!({
                "@odata.id": TASK,
                "@odata.type": "#Task.v1_7_0.Task",
                "Id": "95",
                "Name": "Task 95",
                "TaskState": state,
                "Payload": {"HttpHeaders": headers},
            })
        };
        let bmc = Fixture::new("NVIDIA", "GB200 NVL", "System_0", "HGX_BMC_0")
            .document(
                "/redfish/v1",
                json!({
                    "@odata.id": "/redfish/v1",
                    "Id": "RootService",
                    "Name": "Root Service",
                    "ComponentIntegrity": {"@odata.id": COLLECTION},
                    "Links": {"Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}},
                }),
            )
            .document(
                COLLECTION,
                json!({
                    "@odata.id": COLLECTION,
                    "@odata.type": "#ComponentIntegrityCollection.ComponentIntegrityCollection",
                    "Name": "Component Integrity",
                    "Members": [{"@odata.id": COMPONENT}],
                }),
            )
            .document(
                COMPONENT,
                json!({
                    "@odata.id": COMPONENT,
                    "@odata.type": "#ComponentIntegrity.v1_2_0.ComponentIntegrity",
                    "Id": "HGX_IRoT_GPU_0",
                    "Name": "GPU 0 root of trust",
                    "ComponentIntegrityEnabled": true,
                    "ComponentIntegrityType": "SPDM",
                    "ComponentIntegrityTypeVersion": "1.1.0",
                    "TargetComponentURI": "/redfish/v1/Chassis/HGX_GPU_0",
                    "Actions": {"#ComponentIntegrity.SPDMGetSignedMeasurements": {"target": TARGET}},
                }),
            )
            .respond(
                Method::POST,
                TARGET,
                StatusCode::OK,
                Some(task("Running", json!([]))),
            )
            .document(TASK, task("Completed", json!([format!("Location: {DATA}")])))
            .document(
                DATA,
                json!({
                    "HashingAlgorithm": "TPM_ALG_SHA_384",
                    "SignedMeasurements": "bWVhc3VyZW1lbnRz",
                    "SigningAlgorithm": "TPM_ALG_ECDSA_ECC_NIST_P384",
                    "Version": "1.1",
                }),
            )
            .build()
            .await;
        let cx = bmc.lazy_cx();

        let progress = StandardAttestation
            .request_evidence(&cx, "HGX_IRoT_GPU_0", &[0xab, 0xcd])
            .await
            .expect("request accepted");
        let pending = PendingEvidence {
            uri: TASK.to_string().into(),
            retry_after_seconds: None,
        };
        assert_eq!(progress, EvidenceProgress::Pending(pending.clone()));
        assert_eq!(body(&bmc.writes()[0]), json!({"Nonce": "abcd"}));

        assert_eq!(
            StandardAttestation
                .poll_evidence(&cx, "HGX_IRoT_GPU_0", &pending)
                .await,
            Ok(EvidenceProgress::Ready(AttestationEvidence {
                hashing_algorithm: "TPM_ALG_SHA_384".to_string(),
                signed_measurements: "bWVhc3VyZW1lbnRz".to_string(),
                signing_algorithm: "TPM_ALG_ECDSA_ECC_NIST_P384".to_string(),
                version: "1.1".to_string(),
            }))
        );
    }
}
