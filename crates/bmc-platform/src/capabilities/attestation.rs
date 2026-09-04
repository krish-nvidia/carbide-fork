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
use nv_redfish::core::Bmc;
use nv_redfish::schema::software_inventory::SoftwareInventory;
use serde::{Deserialize, Serialize};

use crate::{DriverOutcome, OpCx, PlatformError};

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

/// Collection of hardware attestation evidence.
#[async_trait]
pub trait Attestation<B: Bmc>: Send + Sync {
    async fn components(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<ComponentIntegritySummary>, PlatformError>;

    async fn firmware_for_component(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<Arc<SoftwareInventory>, PlatformError>;

    async fn ca_certificate(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<CaCertificate, PlatformError>;

    async fn trigger_evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
        nonce: &[u8],
    ) -> Result<DriverOutcome, PlatformError>;

    async fn evidence(
        &self,
        cx: &OpCx<'_, B>,
        component_id: &str,
    ) -> Result<AttestationEvidence, PlatformError>;
}
