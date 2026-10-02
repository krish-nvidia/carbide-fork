/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::sync::Arc;

use async_trait::async_trait;
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::schema::software_inventory::SoftwareInventory;
use serde::{Deserialize, Serialize};

use crate::{OpCx, PlatformError};

/// One ComponentIntegrity resource as listed for attestation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentIntegritySummary {
    /// Redfish `ComponentIntegrity` resource id.
    pub id: String,
    pub name: String,
    /// `ComponentIntegrityEnabled` as reported.
    pub enabled: bool,
    /// `ComponentIntegrityType`, for example `SPDM`.
    pub component_type: String,
    /// `ComponentIntegrityTypeVersion`, the protocol version string.
    pub component_type_version: String,
}

/// The CA certificate a ComponentIntegrity responder authenticates with.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaCertificate {
    /// Certificate body in the encoding named by `certificate_type` (usually PEM).
    pub certificate_string: String,
    /// Redfish `CertificateType`, for example `PEM`.
    pub certificate_type: String,
    /// Redfish `CertificateUsageTypes`.
    pub certificate_usage_types: Vec<String>,
    /// Certificate resource id.
    pub id: String,
    pub name: String,
    /// SPDM certificate slot the responder presented.
    pub slot_id: u16,
}

/// Signed measurements retrieved for a component.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttestationEvidence {
    /// SPDM hashing algorithm name.
    pub hashing_algorithm: String,
    /// Base64 SPDM `MEASUREMENTS` response as returned by the BMC.
    pub signed_measurements: String,
    /// SPDM signing algorithm name.
    pub signing_algorithm: String,
    /// SPDM protocol version of the evidence.
    pub version: String,
}

/// A signed-measurements request the BMC is still running.
///
/// Persist it between polls: the BMC can move the operation to a new URI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingEvidence {
    /// Task Monitor or Task the next poll reads.
    pub uri: ODataId,
    /// Poll interval the BMC suggested, if any.
    pub retry_after_seconds: Option<u64>,
}

/// Signed measurements, or the request still producing them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "details", rename_all = "snake_case")]
pub enum EvidenceProgress {
    /// The measurements are available. A BMC can serve them from a URI shared
    /// between requests, so check them against the request nonce.
    Ready(AttestationEvidence),
    /// The BMC is still collecting; poll again with this request.
    Pending(PendingEvidence),
}

/// Collection of hardware attestation evidence.
///
/// Every operation defaults to delegating to [`Self::standard`], so a driver
/// implements only the operations its platform deviates on.
#[async_trait]
pub trait Attestation<B: Bmc>: Send + Sync {
    /// The driver every operation this driver does not implement delegates to.
    ///
    /// Vendor and model drivers return the capability's standard driver and
    /// implement only their deviations. The standard driver implements every
    /// operation and returns `self`.
    fn standard(&self) -> &dyn Attestation<B>;

    async fn components(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<ComponentIntegritySummary>, PlatformError> {
        self.standard().components(cx).await
    }

    async fn firmware_for_component(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<Arc<SoftwareInventory>, PlatformError> {
        self.standard()
            .firmware_for_component(cx, component_id)
            .await
    }

    async fn ca_certificate(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<CaCertificate, PlatformError> {
        self.standard().ca_certificate(cx, component_id).await
    }

    /// Requests signed measurements over `nonce`.
    async fn request_evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
        nonce: &[u8],
    ) -> Result<EvidenceProgress, PlatformError> {
        self.standard()
            .request_evidence(cx, component_id, nonce)
            .await
    }

    /// Polls a pending signed-measurements request once.
    async fn poll_evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
        pending: &PendingEvidence,
    ) -> Result<EvidenceProgress, PlatformError> {
        self.standard()
            .poll_evidence(cx, component_id, pending)
            .await
    }
}
