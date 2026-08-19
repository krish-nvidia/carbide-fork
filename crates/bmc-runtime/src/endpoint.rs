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

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use bmc_platform::{
    Accounts, Attestation, Bios, BmcControl, BootOrder, Capability, Console, Dpu, Firmware,
    IpmiOps, Lockdown, OpCx, PlatformIdentity, Power, SecureBoot, Storage,
};
use carbide_utils::redfish::BmcAccessInfo;
use mac_address::MacAddress;
use nv_redfish::{Bmc, ServiceRoot};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ConnectError, DispatchError, DriverRegistry, ResolvedSelection, RuleSet,
    project_platform_identity,
};

/// Secret-free, serializable identity of a BMC access endpoint.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BmcRef {
    address: SocketAddr,
    mac_address: MacAddress,
}

impl BmcRef {
    /// Creates a secret-free endpoint reference.
    pub const fn new(address: SocketAddr, mac_address: MacAddress) -> Self {
        Self {
            address,
            mac_address,
        }
    }

    /// Returns the concrete BMC socket address required by the client pool.
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Returns the BMC MAC address used for credential lookup.
    pub const fn mac_address(&self) -> MacAddress {
        self.mac_address
    }
}

impl TryFrom<&BmcAccessInfo> for BmcRef {
    type Error = BmcRefError;

    fn try_from(access: &BmcAccessInfo) -> Result<Self, Self::Error> {
        let host = access
            .host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(&access.host);
        let ip = host
            .parse::<IpAddr>()
            .map_err(|_| BmcRefError::InvalidIpAddress(access.host.clone()))?;
        Ok(Self::new(
            SocketAddr::new(ip, access.port.unwrap_or(443)),
            access.mac_address,
        ))
    }
}

/// Failure to convert generic Redfish access data into a pool endpoint.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BmcRefError {
    /// The configured host is not an IP literal.
    #[error("BMC host must be an IP literal for pooled access: {0}")]
    InvalidIpAddress(String),
}

/// An unconnected endpoint with access coordinates but no credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BmcEndpoint {
    reference: BmcRef,
}

impl BmcEndpoint {
    /// Creates an endpoint from a concrete, secret-free BMC reference.
    pub const fn new(reference: BmcRef) -> Self {
        Self { reference }
    }

    /// Returns the endpoint's secret-free reference.
    pub const fn reference(&self) -> &BmcRef {
        &self.reference
    }
}

impl TryFrom<BmcAccessInfo> for BmcEndpoint {
    type Error = BmcRefError;

    fn try_from(access: BmcAccessInfo) -> Result<Self, Self::Error> {
        BmcRef::try_from(&access).map(Self::new)
    }
}

/// A live Redfish endpoint with projected identity and selected drivers.
pub struct ConnectedBmc<B: Bmc> {
    endpoint: BmcEndpoint,
    service_root: Arc<ServiceRoot<B>>,
    identity: PlatformIdentity,
    selection: ResolvedSelection,
    registry: Arc<DriverRegistry<B>>,
    ipmi: Option<Arc<dyn IpmiOps>>,
}

impl<B: Bmc> ConnectedBmc<B> {
    /// Projects live identity and resolves a unique driver rule.
    ///
    /// The transport has already been constructed by the caller from a
    /// transient credential lease. No credential is retained by this handle.
    pub async fn connect(
        endpoint: BmcEndpoint,
        service_root: Arc<ServiceRoot<B>>,
        rules: &RuleSet,
        registry: Arc<DriverRegistry<B>>,
        ipmi: Option<Arc<dyn IpmiOps>>,
    ) -> Result<Self, ConnectError> {
        let identity = project_platform_identity(service_root.clone()).await?;
        let selection = rules.resolve(&identity)?;
        Ok(Self {
            endpoint,
            service_root,
            identity,
            selection,
            registry,
            ipmi,
        })
    }

    /// Returns the unconnected endpoint metadata.
    pub const fn endpoint(&self) -> &BmcEndpoint {
        &self.endpoint
    }

    /// Returns the live generic Redfish service root.
    pub const fn service_root(&self) -> &Arc<ServiceRoot<B>> {
        &self.service_root
    }

    /// Returns the live identity snapshot used for selection.
    pub const fn identity(&self) -> &PlatformIdentity {
        &self.identity
    }

    /// Returns the deterministic selection decision.
    pub const fn selection(&self) -> &ResolvedSelection {
        &self.selection
    }

    /// Builds an operation context for a stateless capability driver.
    pub fn operation_context(&self) -> OpCx<'_, B> {
        let context = OpCx::new(self.service_root.as_ref());
        match self.ipmi.as_deref() {
            Some(ipmi) => context.with_ipmi(ipmi),
            None => context,
        }
    }

    /// Resolves the selected host-power implementation.
    pub fn power(&self) -> Result<Arc<dyn Power<B>>, DispatchError> {
        self.registry
            .power(self.selection.drivers().get(Capability::Power))
    }

    /// Resolves the selected BMC-control implementation.
    pub fn bmc_control(&self) -> Result<Arc<dyn BmcControl<B>>, DispatchError> {
        self.registry
            .bmc_control(self.selection.drivers().get(Capability::BmcControl))
    }

    /// Resolves the selected BIOS implementation.
    pub fn bios(&self) -> Result<Arc<dyn Bios<B>>, DispatchError> {
        self.registry
            .bios(self.selection.drivers().get(Capability::Bios))
    }

    /// Resolves the selected boot-order implementation.
    pub fn boot_order(&self) -> Result<Arc<dyn BootOrder<B>>, DispatchError> {
        self.registry
            .boot_order(self.selection.drivers().get(Capability::BootOrder))
    }

    /// Resolves the selected Secure Boot implementation.
    pub fn secure_boot(&self) -> Result<Arc<dyn SecureBoot<B>>, DispatchError> {
        self.registry
            .secure_boot(self.selection.drivers().get(Capability::SecureBoot))
    }

    /// Resolves the selected lockdown implementation.
    pub fn lockdown(&self) -> Result<Arc<dyn Lockdown<B>>, DispatchError> {
        self.registry
            .lockdown(self.selection.drivers().get(Capability::Lockdown))
    }

    /// Resolves the selected account-management implementation.
    pub fn accounts(&self) -> Result<Arc<dyn Accounts<B>>, DispatchError> {
        self.registry
            .accounts(self.selection.drivers().get(Capability::Accounts))
    }

    /// Resolves the selected firmware implementation.
    pub fn firmware(&self) -> Result<Arc<dyn Firmware<B>>, DispatchError> {
        self.registry
            .firmware(self.selection.drivers().get(Capability::Firmware))
    }

    /// Resolves the selected storage implementation.
    pub fn storage(&self) -> Result<Arc<dyn Storage<B>>, DispatchError> {
        self.registry
            .storage(self.selection.drivers().get(Capability::Storage))
    }

    /// Resolves the selected DPU implementation.
    pub fn dpu(&self) -> Result<Arc<dyn Dpu<B>>, DispatchError> {
        self.registry
            .dpu(self.selection.drivers().get(Capability::Dpu))
    }

    /// Resolves the selected attestation implementation.
    pub fn attestation(&self) -> Result<Arc<dyn Attestation<B>>, DispatchError> {
        self.registry
            .attestation(self.selection.drivers().get(Capability::Attestation))
    }

    /// Resolves the selected console implementation.
    pub fn console(&self) -> Result<Arc<dyn Console<B>>, DispatchError> {
        self.registry
            .console(self.selection.drivers().get(Capability::Console))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmc_ref_round_trip_contains_only_access_identity() {
        let reference = BmcRef::new(
            "192.0.2.10:8443".parse().expect("valid socket address"),
            MacAddress::new([2, 0, 0, 0, 0, 1]),
        );

        let encoded = serde_json::to_string(&reference).expect("BMC reference serializes");
        assert_eq!(
            serde_json::from_str::<BmcRef>(&encoded).expect("BMC reference deserializes"),
            reference
        );
        assert!(encoded.contains("192.0.2.10"));
        assert!(!encoded.to_ascii_lowercase().contains("password"));
        assert!(!encoded.to_ascii_lowercase().contains("token"));
        assert!(!encoded.to_ascii_lowercase().contains("credential"));
    }

    #[test]
    fn endpoint_uses_canonical_access_info_without_credentials() {
        let endpoint = BmcEndpoint::try_from(BmcAccessInfo {
            host: "192.0.2.11".to_string(),
            port: None,
            mac_address: MacAddress::new([2, 0, 0, 0, 0, 2]),
        })
        .expect("IP endpoint converts");

        assert_eq!(
            endpoint.reference().address(),
            "192.0.2.11:443".parse().expect("valid socket address")
        );
        assert_eq!(
            endpoint.reference().mac_address(),
            MacAddress::new([2, 0, 0, 0, 0, 2])
        );
    }

    #[test]
    fn endpoint_rejects_hostname_without_dns_resolution() {
        assert_eq!(
            BmcEndpoint::try_from(BmcAccessInfo {
                host: "bmc.example.test".to_string(),
                port: None,
                mac_address: MacAddress::new([2, 0, 0, 0, 0, 2]),
            }),
            Err(BmcRefError::InvalidIpAddress(
                "bmc.example.test".to_string()
            ))
        );
    }
}
