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

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{IpmiOps, PlatformError};
use carbide_ipmi::IPMITool;
use carbide_secrets::credentials::CredentialKey;
use carbide_uuid::machine::MachineId;

type ErrorMapper = dyn Fn(String) -> PlatformError + Send + Sync;

/// Per-endpoint adapter exposing only the IPMI operations allowed to drivers.
///
/// `carbide-ipmi` retains responsibility for reading the credential identified
/// by `credential_key`; neither the secret nor a secret-bearing lease is stored
/// by this adapter.
pub struct EndpointIpmiOps {
    tool: Arc<dyn IPMITool>,
    machine_id: MachineId,
    address: SocketAddr,
    credential_key: CredentialKey,
    map_error: Arc<ErrorMapper>,
}

impl EndpointIpmiOps {
    /// Creates an adapter that normalizes opaque IPMI failures as unreachable.
    pub fn new(
        tool: Arc<dyn IPMITool>,
        machine_id: MachineId,
        address: SocketAddr,
        credential_key: CredentialKey,
    ) -> Self {
        Self {
            tool,
            machine_id,
            address,
            credential_key,
            map_error: Arc::new(|_| PlatformError::Unreachable),
        }
    }

    /// Overrides transport-specific IPMI error classification.
    ///
    /// The mapper receives rendered error text. It must avoid putting secrets
    /// into a returned serializable [`PlatformError`].
    pub fn with_error_mapper(
        mut self,
        map_error: impl Fn(String) -> PlatformError + Send + Sync + 'static,
    ) -> Self {
        self.map_error = Arc::new(map_error);
        self
    }

    fn map_error(&self, error: impl ToString) -> PlatformError {
        (self.map_error)(error.to_string())
    }
}

#[async_trait]
impl IpmiOps for EndpointIpmiOps {
    async fn bmc_cold_reset(&self) -> Result<(), PlatformError> {
        self.tool
            .bmc_cold_reset(self.address, &self.credential_key)
            .await
            .map_err(|error| self.map_error(error))
    }

    async fn chassis_power_reset(&self) -> Result<(), PlatformError> {
        self.tool
            .restart(&self.machine_id, self.address, false, &self.credential_key)
            .await
            .map_err(|error| self.map_error(error))
    }

    async fn dpu_legacy_power_reset(&self) -> Result<(), PlatformError> {
        self.tool
            .restart(&self.machine_id, self.address, true, &self.credential_key)
            .await
            .map_err(|error| self.map_error(error))
    }
}
