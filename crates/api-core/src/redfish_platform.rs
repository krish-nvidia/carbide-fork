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

//! Wiring for the Redfish platform service (`carbide-redfish-platform-runtime`).
//!
//! Adapts NICo's credential store to the runtime's [`BmcCredentialProvider`]
//! (BMC MAC -> username/password, exactly as the libredfish path resolves
//! credentials today) and builds the `Arc<dyn RedfishPlatformService>` the
//! binary injects into the state controllers.

use std::sync::Arc;

use async_trait::async_trait;
use carbide_redfish_platform_api::RedfishError;
use carbide_redfish_platform_api::model::BmcRef;
use carbide_redfish_platform_api::service::RedfishPlatformService;
use carbide_redfish_platform_runtime::{BmcCredentialProvider, BmcCredentials, build_runtime};
use carbide_secrets::credentials::{
    BmcCredentialType, CredentialKey, CredentialReader, Credentials,
};

/// Resolves BMC credentials from NICo's credential store keyed by BMC MAC.
struct CredentialReaderProvider {
    reader: Arc<dyn CredentialReader>,
}

#[async_trait]
impl BmcCredentialProvider for CredentialReaderProvider {
    async fn credentials_for(&self, bmc: &BmcRef) -> Result<BmcCredentials, RedfishError> {
        let mac = bmc
            .mac_address
            .ok_or_else(|| RedfishError::MissingCredentials {
                address: Some(bmc.address),
                detail: "BmcRef has no MAC address to resolve credentials".to_string(),
            })?;
        let key = CredentialKey::BmcCredentials {
            credential_type: BmcCredentialType::BmcRoot {
                bmc_mac_address: mac,
            },
        };
        let creds = self
            .reader
            .get_credentials(&key)
            .await
            .map_err(|e| RedfishError::MissingCredentials {
                address: Some(bmc.address),
                detail: e.to_string(),
            })?
            .ok_or_else(|| RedfishError::MissingCredentials {
                address: Some(bmc.address),
                detail: format!("no BMC root credentials for {mac}"),
            })?;
        match creds {
            Credentials::UsernamePassword { username, password } => {
                Ok(BmcCredentials { username, password })
            }
        }
    }
}

/// A provider that always fails -- used to populate the service field in
/// contexts that do not exercise it yet (e.g. tests not migrated off the
/// libredfish path).
#[cfg(any(test, feature = "test-support"))]
struct UnavailableCredentials;

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl BmcCredentialProvider for UnavailableCredentials {
    async fn credentials_for(&self, bmc: &BmcRef) -> Result<BmcCredentials, RedfishError> {
        Err(RedfishError::MissingCredentials {
            address: Some(bmc.address),
            detail: "Redfish platform service is not configured in this context".to_string(),
        })
    }
}

/// Build the platform service backed by NICo's credential store.
pub fn build_platform_service(
    reader: Arc<dyn CredentialReader>,
) -> Arc<dyn RedfishPlatformService> {
    build_runtime(Arc::new(CredentialReaderProvider { reader }))
}

/// Build a platform service whose credential resolution always fails. For
/// wiring the service field where it is not yet exercised.
#[cfg(any(test, feature = "test-support"))]
pub fn unconfigured_platform_service() -> Arc<dyn RedfishPlatformService> {
    build_runtime(Arc::new(UnavailableCredentials))
}
