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

//! Reaching a BMC with the CLI credentials: discovery of the
//! [`PlatformIdentity`] selection rules match on, fresh logins and reads,
//! and the `bmc-runtime` connection checks drive drivers through.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use bmc_platform::{
    ChassisIdentity, ClassifyBmcError, Fetched, FirmwareInventoryIdentity, IpmiOps,
    ManagerIdentity, PlatformError, PlatformIdentity, ServiceRootIdentity, SystemIdentity,
};
use bmc_runtime::{
    BmcRef, ConnectedBmc, ConnectionManager, CredentialLease, CredentialRequest, EndpointIpmiOps,
    RuntimeCredentialProvider,
};
use carbide_redfish::nv_redfish::{NvRedfishClientPool, NvRedfishConnection, RedfishBmc};
use carbide_secrets::SecretsError;
use carbide_secrets::credentials::{
    BmcCredentialType, CredentialKey, CredentialReader, Credentials,
};
use carbide_uuid::machine::{MachineId, MachineIdSource, MachineType};
use mac_address::MacAddress;
use nv_redfish::Bmc;
use nv_redfish::bmc_http::BmcCredentials;
use nv_redfish::chassis::Chassis;
use nv_redfish::computer_system::ComputerSystem;
use nv_redfish::core::ODataId;
use nv_redfish::manager::Manager;
use serde::Deserialize;
use serde_json::{Map, Value};

/// The MAC address in the validator's credential key. `BmcRef` keys
/// credentials by BMC MAC, but the validator issues the CLI credentials for
/// every key, so the address is never looked up.
const CREDENTIAL_MAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0];

/// Chassis whose fetch can hang BlueField BMC firmware; site exploration
/// skips them too, and no selection rule reads them.
const HANGING_CHASSIS: &[&str] = &["Bluefield_ERoT", "BlueField_IRoT_NIC_0"];

/// How long discovery may take before it gives up.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(180);

/// The name the validator requests credentials under.
const CALLER: &str = "bmc-validate";

pub(crate) fn credential_key() -> CredentialKey {
    CredentialKey::BmcCredentials {
        credential_type: BmcCredentialType::BmcRoot {
            bmc_mac_address: MacAddress::new(CREDENTIAL_MAC),
        },
    }
}

/// One BMC, reached as one account.
#[derive(Clone)]
pub(crate) struct Target {
    pool: Arc<NvRedfishClientPool>,
    address: SocketAddr,
    username: String,
    password: String,
}

impl Target {
    pub(crate) fn new(address: SocketAddr, username: &str, password: &str) -> Self {
        Self {
            pool: carbide_redfish::nv_redfish::new_pool(Arc::new(ArcSwap::from_pointee(None))),
            address,
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    /// The same BMC reached as another account.
    pub(crate) fn as_account(&self, username: &str, password: &str) -> Self {
        Self {
            username: username.to_string(),
            password: password.to_string(),
            ..self.clone()
        }
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn username(&self) -> &str {
        &self.username
    }

    fn credentials(&self) -> BmcCredentials {
        BmcCredentials::new(self.username.clone(), self.password.clone())
    }

    /// A connection over a ServiceRoot read now, so nothing it reads was
    /// cached before an earlier change.
    pub(crate) async fn fresh_connection(&self) -> Result<NvRedfishConnection, PlatformError> {
        self.pool.invalidate_service_roots_for_bmc(self.address);
        self.pool
            .connection_with_bmc_credentials(self.address, self.credentials())
            .await
            .map_err(PlatformError::from_redfish)
    }

    /// Reads an authenticated resource over a fresh connection; a factory
    /// BMC refuses it with `PasswordChangeRequired`.
    pub(crate) async fn log_in(&self) -> Result<(), PlatformError> {
        self.fresh_connection()
            .await?
            .bmc
            .get::<Fetched<Map<String, Value>>>(&ODataId::from("/redfish/v1/Managers".to_string()))
            .await
            .map(drop)
            .map_err(ClassifyBmcError::classify)
    }

    /// The ServiceRoot part of the identity, the only part a factory BMC
    /// serves.
    pub(crate) async fn service_root_identity(&self) -> Result<ServiceRootIdentity, String> {
        let connection = self
            .fresh_connection()
            .await
            .map_err(|error| format!("connecting: {error}"))?;
        service_root_identity(&connection).await
    }

    /// Builds the identity the way site exploration selects resources: the
    /// first ComputerSystem unless a later one has a BIOS, the first Manager,
    /// every Chassis, and the firmware inventory with exploration's version
    /// normalization.
    pub(crate) async fn discover(&self) -> Result<PlatformIdentity, String> {
        tokio::time::timeout(DISCOVERY_TIMEOUT, async {
            let connection = self
                .fresh_connection()
                .await
                .map_err(|error| format!("connecting: {error}"))?;
            let service_root = service_root_identity(&connection).await?;
            let lenovo_xcc = service_root.vendor.as_deref() == Some("Lenovo")
                && !service_root.oem_keys.iter().any(|key| key == "Ami");
            Ok(PlatformIdentity {
                system: system_identity(&connection)
                    .await
                    .map_err(|error| format!("reading systems: {error}"))?,
                manager: manager_identity(&connection)
                    .await
                    .map_err(|error| format!("reading managers: {error}"))?,
                chassis: chassis_identities(&connection)
                    .await
                    .map_err(|error| format!("reading chassis: {error}"))?,
                firmware_inventory: firmware_inventory(&connection, lenovo_xcc)
                    .await
                    .map_err(|error| format!("reading firmware inventory: {error}"))?,
                service_root,
            })
        })
        .await
        .map_err(|_| format!("discovery did not finish within {DISCOVERY_TIMEOUT:?}"))?
    }

    /// Connects through `bmc-runtime` with this account, attaching
    /// `ipmitool` with the same account when `ipmi` is set.
    pub(crate) async fn connect(
        &self,
        endpoint: BmcRef,
        ipmi: bool,
    ) -> Result<ConnectedBmc<RedfishBmc>, String> {
        let ipmi = ipmi.then(|| self.ipmi());
        ConnectionManager::new(
            self.pool.clone(),
            Arc::new(StaticCredentials(self.credentials())),
            CALLER.to_string(),
        )
        .map_err(|error| error.to_string())?
        .connect(endpoint, ipmi)
        .await
        .map_err(|error| format!("connecting through bmc-runtime: {error}"))
    }

    fn ipmi(&self) -> Arc<dyn IpmiOps> {
        let tool = carbide_ipmi::tool(
            Arc::new(StaticCredentialReader(Credentials::new(
                &self.username,
                &self.password,
            ))),
            None,
        );
        // The machine id only labels ipmitool errors; the validator has no machine record.
        let machine_id = MachineId::new(
            MachineIdSource::ProductBoardChassisSerial,
            [0; 32],
            MachineType::Host,
        );
        Arc::new(EndpointIpmiOps::new(
            tool,
            machine_id,
            SocketAddr::new(self.address.ip(), carbide_ipmi::DEFAULT_IPMI_PORT),
            credential_key(),
        ))
    }
}

/// Issues the CLI credentials to `bmc-runtime` for every request.
struct StaticCredentials(BmcCredentials);

#[async_trait]
impl RuntimeCredentialProvider for StaticCredentials {
    async fn issue(&self, _request: &CredentialRequest) -> Result<CredentialLease, PlatformError> {
        Ok(CredentialLease::new(self.0.clone()))
    }
}

/// Hands the CLI credentials to `ipmitool` for every key.
struct StaticCredentialReader(Credentials);

#[async_trait]
impl CredentialReader for StaticCredentialReader {
    async fn get_credentials(
        &self,
        _key: &CredentialKey,
    ) -> Result<Option<Credentials>, SecretsError> {
        Ok(Some(self.0.clone()))
    }
}

/// The ServiceRoot fields selection reads that the typed root does not expose.
#[derive(Deserialize)]
struct ServiceRootDocument {
    #[serde(rename = "Vendor")]
    vendor: Option<String>,
    #[serde(rename = "Product")]
    product: Option<String>,
    #[serde(rename = "Oem", default)]
    oem: Map<String, Value>,
}

async fn service_root_identity(
    connection: &NvRedfishConnection,
) -> Result<ServiceRootIdentity, String> {
    let root = connection
        .bmc
        .get::<Fetched<ServiceRootDocument>>(&ODataId::from("/redfish/v1".to_string()))
        .await
        .map_err(|error| format!("reading the service root: {}", error.classify()))?;
    Ok(ServiceRootIdentity {
        vendor: root.vendor.clone(),
        product: root.product.clone(),
        oem_keys: root.oem.keys().cloned().collect(),
    })
}

/// The ComputerSystem drivers act on: the first, unless a later one has a
/// BIOS. Power shelves list no systems; their BMC answers the collection
/// with a 404.
pub(crate) async fn system(
    connection: &NvRedfishConnection,
) -> Result<Option<ComputerSystem<RedfishBmc>>, PlatformError> {
    let Some(collection) = connection
        .service_root
        .systems()
        .await
        .map_err(PlatformError::from_redfish)?
    else {
        return Ok(None);
    };
    let members = match collection
        .members()
        .await
        .map_err(PlatformError::from_redfish)
    {
        Ok(members) => members,
        Err(PlatformError::Bmc { status: 404, .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut members = members.into_iter();
    let Some(first) = members.next() else {
        return Ok(None);
    };
    Ok(Some(
        members
            .find(|system| system.raw().bios.is_some())
            .unwrap_or(first),
    ))
}

/// The Manager drivers act on: the first.
pub(crate) async fn manager(
    connection: &NvRedfishConnection,
) -> Result<Option<Manager<RedfishBmc>>, PlatformError> {
    let Some(collection) = connection
        .service_root
        .managers()
        .await
        .map_err(PlatformError::from_redfish)?
    else {
        return Ok(None);
    };
    Ok(collection
        .members()
        .await
        .map_err(PlatformError::from_redfish)?
        .into_iter()
        .next())
}

async fn system_identity(
    connection: &NvRedfishConnection,
) -> Result<Option<SystemIdentity>, PlatformError> {
    Ok(system(connection).await?.map(|system| {
        let raw = system.raw();
        SystemIdentity {
            id: raw.id.clone(),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            sku: raw.sku.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
            bios_version: raw.bios_version.clone().flatten(),
        }
    }))
}

async fn manager_identity(
    connection: &NvRedfishConnection,
) -> Result<Option<ManagerIdentity>, PlatformError> {
    Ok(manager(connection).await?.map(|manager| {
        let raw = manager.raw();
        ManagerIdentity {
            id: raw.id.clone(),
            model: raw.model.clone().flatten(),
            firmware: raw.firmware_version.clone().flatten(),
        }
    }))
}

/// Every Chassis except those whose fetch can hang the BMC.
pub(crate) async fn chassis(
    connection: &NvRedfishConnection,
) -> Result<Vec<Chassis<RedfishBmc>>, PlatformError> {
    let links = connection
        .service_root
        .chassis_links()
        .await
        .map_err(PlatformError::from_redfish)?
        .unwrap_or_default();
    let mut chassis = Vec::with_capacity(links.len());
    for link in links {
        if link
            .odata_id()
            .last_segment()
            .is_some_and(|id| HANGING_CHASSIS.contains(&id))
        {
            continue;
        }
        chassis.push(link.upgrade().await.map_err(PlatformError::from_redfish)?);
    }
    Ok(chassis)
}

async fn chassis_identities(
    connection: &NvRedfishConnection,
) -> Result<Vec<ChassisIdentity>, PlatformError> {
    Ok(chassis(connection)
        .await?
        .iter()
        .map(|member| {
            let raw = member.raw();
            ChassisIdentity {
                id: raw.id.clone(),
                manufacturer: raw.manufacturer.clone().flatten(),
                model: raw.model.clone().flatten(),
                part_number: raw.part_number.clone().flatten(),
            }
        })
        .collect())
}

async fn firmware_inventory(
    connection: &NvRedfishConnection,
    lenovo_xcc: bool,
) -> Result<Vec<FirmwareInventoryIdentity>, PlatformError> {
    let Some(service) = connection
        .service_root
        .update_service()
        .await
        .map_err(PlatformError::from_redfish)?
    else {
        return Ok(Vec::new());
    };
    let inventories = service
        .firmware_inventories()
        .await
        .map_err(PlatformError::from_redfish)?
        .unwrap_or_default();
    Ok(inventories
        .iter()
        .map(|inventory| {
            let raw = inventory.raw();
            FirmwareInventoryIdentity {
                id: raw.id.clone(),
                version: raw
                    .version
                    .clone()
                    .flatten()
                    .map(|version| normalized_version(&version, lenovo_xcc)),
            }
        })
        .collect())
}

/// Lenovo XCC prefixes most versions with a build id and a dash, and GB200
/// BMC firmware prefixes its version with `GB200Nvl-`.
fn normalized_version(version: &str, lenovo_xcc: bool) -> String {
    let version = version.strip_prefix("GB200Nvl-").unwrap_or(version);
    if lenovo_xcc {
        version
            .split('-')
            .next_back()
            .unwrap_or_default()
            .to_string()
    } else {
        version.to_string()
    }
}
