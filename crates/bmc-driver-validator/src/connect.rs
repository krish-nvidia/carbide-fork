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

//! Connection wiring for the CLI credentials, and discovery of the
//! [`PlatformIdentity`] selection rules match on.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use bmc_platform::{
    ChassisIdentity, ClassifyBmcError, Fetched, FirmwareInventoryIdentity, IpmiOps,
    ManagerIdentity, PlatformError, PlatformIdentity, ServiceRootIdentity, SystemIdentity,
};
use bmc_runtime::{CredentialLease, CredentialRequest, EndpointIpmiOps, RuntimeCredentialProvider};
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
use serde::Deserialize;
use serde_json::{Map, Value};

/// The MAC address in the validator's credential key. `BmcRef` keys
/// credentials by BMC MAC, but the validator issues the CLI credentials for
/// every key, so the address is never looked up.
const CREDENTIAL_MAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0];

/// Chassis whose fetch can hang BlueField BMC firmware; site exploration
/// skips them too, and no selection rule reads them.
const HANGING_CHASSIS: &[&str] = &["Bluefield_ERoT", "BlueField_IRoT_NIC_0"];

pub(crate) fn pool() -> Arc<NvRedfishClientPool> {
    carbide_redfish::nv_redfish::new_pool(Arc::new(ArcSwap::from_pointee(None)))
}

pub(crate) fn credential_key() -> CredentialKey {
    CredentialKey::BmcCredentials {
        credential_type: BmcCredentialType::BmcRoot {
            bmc_mac_address: MacAddress::new(CREDENTIAL_MAC),
        },
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

pub(crate) fn static_credentials(
    credentials: BmcCredentials,
) -> Arc<dyn RuntimeCredentialProvider> {
    Arc::new(StaticCredentials(credentials))
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

pub(crate) fn ipmi(ip: IpAddr, username: &str, password: &str) -> Arc<dyn IpmiOps> {
    let tool = carbide_ipmi::tool(
        Arc::new(StaticCredentialReader(Credentials::new(username, password))),
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
        SocketAddr::new(ip, carbide_ipmi::DEFAULT_IPMI_PORT),
        credential_key(),
    ))
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

/// Builds the identity the way site exploration selects resources: the
/// first ComputerSystem unless a later one has a BIOS, the first Manager,
/// every Chassis, and the firmware inventory with exploration's version
/// normalization.
pub(crate) async fn discover(
    pool: &NvRedfishClientPool,
    address: SocketAddr,
    credentials: BmcCredentials,
) -> Result<PlatformIdentity, String> {
    let connection = pool
        .connection_with_bmc_credentials(address, credentials)
        .await
        .map_err(|error| format!("connecting: {}", PlatformError::from_redfish(error)))?;
    let root = connection
        .bmc
        .get::<Fetched<ServiceRootDocument>>(&ODataId::from("/redfish/v1".to_string()))
        .await
        .map_err(|error| format!("reading the service root: {}", error.classify()))?;
    let service_root = ServiceRootIdentity {
        vendor: root.vendor.clone(),
        product: root.product.clone(),
        oem_keys: root.oem.keys().cloned().collect(),
    };
    let lenovo_xcc = service_root.vendor.as_deref() == Some("Lenovo")
        && !service_root.oem_keys.iter().any(|key| key == "Ami");
    Ok(PlatformIdentity {
        system: system(&connection)
            .await
            .map_err(|error| format!("reading systems: {error}"))?,
        manager: manager(&connection)
            .await
            .map_err(|error| format!("reading managers: {error}"))?,
        chassis: chassis(&connection)
            .await
            .map_err(|error| format!("reading chassis: {error}"))?,
        firmware_inventory: firmware_inventory(&connection, lenovo_xcc)
            .await
            .map_err(|error| format!("reading firmware inventory: {error}"))?,
        service_root,
    })
}

/// Power shelves list no systems; their BMC answers the collection with a 404.
async fn system(connection: &NvRedfishConnection) -> Result<Option<SystemIdentity>, PlatformError> {
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
    Ok(selected_system(members).map(|system| {
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

fn selected_system(members: Vec<ComputerSystem<RedfishBmc>>) -> Option<ComputerSystem<RedfishBmc>> {
    let mut members = members.into_iter();
    let first = members.next()?;
    Some(
        members
            .find(|system| system.raw().bios.is_some())
            .unwrap_or(first),
    )
}

async fn manager(
    connection: &NvRedfishConnection,
) -> Result<Option<ManagerIdentity>, PlatformError> {
    let Some(collection) = connection
        .service_root
        .managers()
        .await
        .map_err(PlatformError::from_redfish)?
    else {
        return Ok(None);
    };
    let members = collection
        .members()
        .await
        .map_err(PlatformError::from_redfish)?;
    Ok(members.first().map(|manager| {
        let raw = manager.raw();
        ManagerIdentity {
            id: raw.id.clone(),
            model: raw.model.clone().flatten(),
            firmware: raw.firmware_version.clone().flatten(),
        }
    }))
}

async fn chassis(connection: &NvRedfishConnection) -> Result<Vec<ChassisIdentity>, PlatformError> {
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
        let member: Chassis<RedfishBmc> =
            link.upgrade().await.map_err(PlatformError::from_redfish)?;
        let raw = member.raw();
        chassis.push(ChassisIdentity {
            id: raw.id.clone(),
            manufacturer: raw.manufacturer.clone().flatten(),
            model: raw.model.clone().flatten(),
            part_number: raw.part_number.clone().flatten(),
        });
    }
    Ok(chassis)
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
