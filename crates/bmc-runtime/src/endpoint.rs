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

use bmc_platform::{DriverMap, IpmiOps, OpCx};
use carbide_secrets::credentials::{BmcCredentialType, CredentialKey};
use carbide_utils::redfish::BmcAccessInfo;
use mac_address::MacAddress;
use nv_redfish::{Bmc, ServiceRoot};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Secret-free, serializable identity of a BMC access endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BmcRef {
    address: SocketAddr,
    mac_address: MacAddress,
    credential_key: CredentialKey,
    driver_map: DriverMap,
}

impl BmcRef {
    /// Creates a secret-free endpoint reference.
    pub fn new(
        address: SocketAddr,
        credential_key: CredentialKey,
        driver_map: DriverMap,
    ) -> Result<Self, BmcRefError> {
        let mac_address = match &credential_key {
            CredentialKey::BmcCredentials {
                credential_type: BmcCredentialType::BmcRoot { bmc_mac_address },
            } => *bmc_mac_address,
            _ => return Err(BmcRefError::UnsupportedCredentialKey),
        };
        Ok(Self {
            address,
            mac_address,
            credential_key,
            driver_map,
        })
    }

    /// Returns the concrete BMC socket address required by the client pool.
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Returns the BMC MAC address used for credential lookup.
    pub const fn mac_address(&self) -> MacAddress {
        self.mac_address
    }

    /// Returns the secret-free key used by credential and IPMI providers.
    pub const fn credential_key(&self) -> &CredentialKey {
        &self.credential_key
    }

    /// Returns the complete driver map persisted during exploration.
    pub const fn driver_map(&self) -> &DriverMap {
        &self.driver_map
    }

    /// Builds a runtime reference from canonical access metadata and persisted selection.
    pub fn from_access_info(
        access: &BmcAccessInfo,
        credential_key: CredentialKey,
        driver_map: DriverMap,
    ) -> Result<Self, BmcRefError> {
        let host = access
            .host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(&access.host);
        let ip = host
            .parse::<IpAddr>()
            .map_err(|_| BmcRefError::InvalidIpAddress(access.host.clone()))?;
        let reference = Self::new(
            SocketAddr::new(ip, access.port.unwrap_or(443)),
            credential_key,
            driver_map,
        )?;
        if reference.mac_address != access.mac_address {
            return Err(BmcRefError::MacAddressMismatch);
        }
        Ok(reference)
    }
}

/// Failure to convert generic Redfish access data into a pool endpoint.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BmcRefError {
    /// The configured host is not an IP literal.
    #[error("BMC host must be an IP literal for pooled access: {0}")]
    InvalidIpAddress(String),
    /// Runtime BMC references currently require per-BMC root credentials.
    #[error("BMC reference must use a per-BMC root credential key")]
    UnsupportedCredentialKey,
    /// Access metadata and credential key refer to different BMCs.
    #[error("BMC access MAC address does not match its credential key")]
    MacAddressMismatch,
}

/// A live Redfish endpoint using the driver map persisted during exploration.
pub struct ConnectedBmc<B: Bmc> {
    endpoint: BmcRef,
    service_root: Arc<ServiceRoot<B>>,
    ipmi: Option<Arc<dyn IpmiOps>>,
}

impl<B: Bmc> ConnectedBmc<B> {
    /// Creates a connected endpoint without repeating exploration or selection.
    pub fn new(
        endpoint: BmcRef,
        service_root: Arc<ServiceRoot<B>>,
        ipmi: Option<Arc<dyn IpmiOps>>,
    ) -> Self {
        Self {
            endpoint,
            service_root,
            ipmi,
        }
    }

    /// Returns the unconnected endpoint metadata.
    pub const fn endpoint(&self) -> &BmcRef {
        &self.endpoint
    }

    /// Returns the live generic Redfish service root.
    pub const fn service_root(&self) -> &Arc<ServiceRoot<B>> {
        &self.service_root
    }

    pub(crate) fn replace_service_root(&mut self, service_root: Arc<ServiceRoot<B>>) {
        self.service_root = service_root;
    }

    /// Builds an operation context for a stateless capability driver.
    pub fn operation_context(&self) -> OpCx<'_, B> {
        let context = OpCx::new(self.service_root.as_ref());
        match self.ipmi.as_deref() {
            Some(ipmi) => context.with_ipmi(ipmi),
            None => context,
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_platform::{CapabilitySelection, DriverMap};
    use carbide_secrets::credentials::{BmcCredentialType, CredentialKey};

    use super::*;

    fn unsupported_driver_map() -> DriverMap {
        let unsupported = CapabilitySelection::Unsupported;
        DriverMap {
            power: unsupported.clone(),
            bmc_control: unsupported.clone(),
            bios: unsupported.clone(),
            boot_order: unsupported.clone(),
            secure_boot: unsupported.clone(),
            lockdown: unsupported.clone(),
            accounts: unsupported.clone(),
            firmware: unsupported.clone(),
            storage: unsupported.clone(),
            dpu: unsupported.clone(),
            attestation: unsupported.clone(),
            console: unsupported,
        }
    }

    fn credential_key(mac_address: MacAddress) -> CredentialKey {
        CredentialKey::BmcCredentials {
            credential_type: BmcCredentialType::BmcRoot {
                bmc_mac_address: mac_address,
            },
        }
    }

    #[test]
    fn bmc_ref_round_trip_contains_only_access_identity() {
        let mac_address = MacAddress::new([2, 0, 0, 0, 0, 1]);
        let driver_map = unsupported_driver_map();
        let reference = BmcRef::new(
            "192.0.2.10:8443".parse().expect("valid socket address"),
            credential_key(mac_address),
            driver_map.clone(),
        )
        .expect("root credential key is valid");

        let encoded = serde_json::to_string(&reference).expect("BMC reference serializes");
        let decoded = serde_json::from_str::<BmcRef>(&encoded).expect("BMC reference deserializes");
        assert_eq!(decoded.address(), reference.address());
        assert_eq!(decoded.mac_address(), mac_address);
        assert_eq!(decoded.driver_map(), &driver_map);
        assert!(encoded.contains("192.0.2.10"));
        assert!(!encoded.to_ascii_lowercase().contains("password"));
        assert!(!encoded.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn endpoint_uses_canonical_access_info_without_credentials() {
        let mac_address = MacAddress::new([2, 0, 0, 0, 0, 2]);
        let access = BmcAccessInfo {
            host: "192.0.2.11".to_string(),
            port: None,
            mac_address,
        };
        let endpoint = BmcRef::from_access_info(
            &access,
            credential_key(mac_address),
            unsupported_driver_map(),
        )
        .expect("IP endpoint converts");

        assert_eq!(
            endpoint.address(),
            "192.0.2.11:443".parse().expect("valid socket address")
        );
        assert_eq!(endpoint.mac_address(), mac_address);
    }

    #[test]
    fn endpoint_rejects_hostname_without_dns_resolution() {
        assert!(matches!(
            BmcRef::from_access_info(
                &BmcAccessInfo {
                    host: "bmc.example.test".to_string(),
                    port: None,
                    mac_address: MacAddress::new([2, 0, 0, 0, 0, 2]),
                },
                credential_key(MacAddress::new([2, 0, 0, 0, 0, 2])),
                unsupported_driver_map(),
            ),
            Err(BmcRefError::InvalidIpAddress(host)) if host == "bmc.example.test"
        ));
    }

    #[test]
    fn bmc_ref_rejects_non_device_credentials_and_mismatched_metadata() {
        let address = "192.0.2.11:443".parse().expect("valid socket address");
        assert!(matches!(
            BmcRef::new(
                address,
                CredentialKey::BmcCredentials {
                    credential_type: BmcCredentialType::SiteWideRoot,
                },
                unsupported_driver_map(),
            ),
            Err(BmcRefError::UnsupportedCredentialKey)
        ));

        let access = BmcAccessInfo {
            host: "192.0.2.11".to_string(),
            port: None,
            mac_address: MacAddress::new([2, 0, 0, 0, 0, 2]),
        };
        assert!(matches!(
            BmcRef::from_access_info(
                &access,
                credential_key(MacAddress::new([2, 0, 0, 0, 0, 3])),
                unsupported_driver_map(),
            ),
            Err(BmcRefError::MacAddressMismatch)
        ));
    }
}
