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

//! Reads of what a driver wrote, from the resources themselves, for
//! settings no driver reads back: nv-redfish traversed from a ServiceRoot
//! read now, through the system and manager the drivers act on.

use std::time::SystemTime;

use bmc_platform::{ClassifyBmcError, Fetched, PlatformError};
use carbide_redfish::nv_redfish::{NvRedfishConnection, RedfishBmc};
use nv_redfish::Bmc;
use nv_redfish::account::AccountServiceConfig;
use nv_redfish::computer_system::ComputerSystem;
use nv_redfish::core::ODataId;
use nv_redfish::manager::Manager;
use nv_redfish::schema::computer_system::{BootSource, BootSourceOverrideEnabled};
use serde_json::{Map, Value};

use super::{Ctx, reported};
use crate::connect;

/// The NTP servers the BMC lists.
pub(super) struct Ntp {
    /// `NTP.ProtocolEnabled`; HPE's OEM `DateTime` has no such flag.
    pub(super) enabled: Option<bool>,
    pub(super) servers: Vec<String>,
}

impl Ntp {
    /// Whether the BMC lists the servers in `wanted` it has room for: as
    /// many as it lists entries, from the front. HPE iLO and Supermicro keep
    /// two.
    pub(super) fn lists(&self, wanted: &[String]) -> bool {
        let kept = self.servers.len().min(wanted.len());
        self.enabled != Some(false)
            && kept > 0
            && wanted[..kept]
                .iter()
                .all(|server| self.servers.contains(server))
    }
}

pub(super) async fn ntp(ctx: &Ctx) -> Result<Ntp, PlatformError> {
    let manager = manager(&connection(ctx).await?).await?;
    if let Some(hpe) = manager.oem_hpe().map_err(PlatformError::from_redfish)? {
        let date_time = hpe.date_time().await.map_err(PlatformError::from_redfish)?;
        let servers = date_time
            .and_then(|date_time| date_time.raw().static_ntp_servers.clone())
            .unwrap_or_default();
        return Ok(Ntp {
            enabled: None,
            servers: servers
                .into_iter()
                .filter(|server| !server.is_empty())
                .collect(),
        });
    }
    let protocol = manager
        .network_protocol()
        .await
        .map_err(PlatformError::from_redfish)?
        .ok_or_else(|| absent("Manager.NetworkProtocol"))?;
    let raw = protocol.raw();
    let ntp = raw.ntp.as_ref();
    Ok(Ntp {
        enabled: ntp.and_then(|ntp| ntp.protocol_enabled.flatten()),
        servers: ntp
            .and_then(|ntp| ntp.ntp_servers.clone().flatten())
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect(),
    })
}

/// The manager's clock and its offset from UTC.
pub(super) struct Clock {
    pub(super) now: Option<SystemTime>,
    pub(super) offset: Option<String>,
}

pub(super) async fn clock(ctx: &Ctx) -> Result<Clock, PlatformError> {
    let raw = manager(&connection(ctx).await?).await?.raw();
    Ok(Clock {
        now: raw
            .date_time
            .flatten()
            .and_then(|now| SystemTime::try_from(now).ok()),
        offset: raw.date_time_local_offset.clone().flatten(),
    })
}

/// `Manager.NetworkProtocol.IPMI.ProtocolEnabled`.
pub(super) async fn ipmi_enabled(ctx: &Ctx) -> Result<Option<bool>, PlatformError> {
    let protocol = manager(&connection(ctx).await?)
        .await?
        .network_protocol()
        .await
        .map_err(PlatformError::from_redfish)?;
    Ok(protocol.and_then(|protocol| {
        protocol
            .raw()
            .ipmi
            .as_ref()
            .and_then(|ipmi| ipmi.protocol_enabled.flatten())
    }))
}

/// The account lockout settings, as name/value fields.
pub(super) async fn lockout_policy(ctx: &Ctx) -> Result<Vec<(String, String)>, PlatformError> {
    let service = connection(ctx)
        .await?
        .service_root
        .account_service(AccountServiceConfig::standard())
        .await
        .map_err(PlatformError::from_redfish)?
        .ok_or_else(|| absent("AccountService"))?;
    let raw = service.raw();
    Ok(vec![
        (
            "lockout threshold".to_string(),
            reported(&raw.account_lockout_threshold.flatten()),
        ),
        (
            "lockout duration".to_string(),
            reported(&raw.account_lockout_duration.flatten()),
        ),
        (
            "counter reset after".to_string(),
            reported(&raw.account_lockout_counter_reset_after),
        ),
    ])
}

/// The system's one-time boot override.
pub(super) struct BootOverride {
    pub(super) target: Option<BootSource>,
    pub(super) enabled: Option<BootSourceOverrideEnabled>,
}

impl BootOverride {
    /// Whether the next boot, and only the next, goes to `target`.
    pub(super) fn is_once(&self, target: BootSource) -> bool {
        self.target == Some(target) && self.enabled == Some(BootSourceOverrideEnabled::Once)
    }
}

pub(super) async fn boot_override(ctx: &Ctx) -> Result<BootOverride, PlatformError> {
    let raw = system(&connection(ctx).await?).await?.raw();
    let boot = raw.boot.as_ref();
    Ok(BootOverride {
        target: boot.and_then(|boot| boot.boot_source_override_target.flatten()),
        enabled: boot.and_then(|boot| boot.boot_source_override_enabled.flatten()),
    })
}

/// `BmcRShim.BmcRShimEnabled` on the resource the manager's `Oem.Nvidia`
/// links, where the driver writes it.
pub(super) async fn bmc_rshim_enabled(ctx: &Ctx) -> Result<Option<bool>, PlatformError> {
    let connection = connection(ctx).await?;
    let manager = get(
        &connection,
        manager(&connection).await?.raw().odata_id.clone(),
    )
    .await?;
    let Some(link) = field(&manager, &["Oem", "Nvidia", "@odata.id"]).and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let resource = get(&connection, ODataId::from(link.to_string())).await?;
    Ok(field(&resource, &["BmcRShim", "BmcRShimEnabled"]).and_then(Value::as_bool))
}

/// The DPU's host privilege level: a BIOS attribute on BlueField-2 and -3,
/// the NVIDIA network adapter's setting on BlueField-4.
pub(super) async fn host_privilege(ctx: &Ctx) -> Result<Option<String>, PlatformError> {
    let connection = connection(ctx).await?;
    if let Some(bios) = system(&connection)
        .await?
        .bios()
        .await
        .map_err(PlatformError::from_redfish)?
    {
        let level = ["HostPrivilegeLevel", "Host Privilege Level"]
            .into_iter()
            .find_map(|name| {
                bios.attribute(name)
                    .and_then(|value| value.str_value().map(str::to_string))
            });
        if level.is_some() {
            return Ok(level);
        }
    }
    for chassis in connect::chassis(&connection).await? {
        let adapters = chassis
            .network_adapters()
            .await
            .map_err(PlatformError::from_redfish)?
            .unwrap_or_default();
        for adapter in adapters {
            let Some(nvidia) = adapter.oem_nvidia().map_err(PlatformError::from_redfish)? else {
                continue;
            };
            let config = nvidia
                .host_privilege_config()
                .await
                .map_err(PlatformError::from_redfish)?;
            return Ok(config.and_then(|config| {
                config
                    .raw()
                    .privilege_mode
                    .flatten()
                    .map(|mode| reported(&Some(mode)))
            }));
        }
    }
    Ok(None)
}

async fn connection(ctx: &Ctx) -> Result<NvRedfishConnection, PlatformError> {
    ctx.target.fresh_connection().await
}

async fn system(
    connection: &NvRedfishConnection,
) -> Result<ComputerSystem<RedfishBmc>, PlatformError> {
    connect::system(connection)
        .await?
        .ok_or_else(|| absent("ComputerSystem"))
}

async fn manager(connection: &NvRedfishConnection) -> Result<Manager<RedfishBmc>, PlatformError> {
    connect::manager(connection)
        .await?
        .ok_or_else(|| absent("Manager"))
}

/// A resource as JSON, for fields the typed schema does not carry.
async fn get(
    connection: &NvRedfishConnection,
    uri: ODataId,
) -> Result<Map<String, Value>, PlatformError> {
    connection
        .bmc
        .get::<Fetched<Map<String, Value>>>(&uri)
        .await
        .map(|document| (*document).clone())
        .map_err(ClassifyBmcError::classify)
}

/// The value at `path` in `document`.
fn field<'a>(document: &'a Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    rest.iter()
        .try_fold(document.get(*first)?, |value, key| value.get(key))
}

fn absent(resource: &str) -> PlatformError {
    PlatformError::InvalidResponse {
        message: format!("the BMC lists no {resource}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMC with fewer slots than servers given keeps the first ones; one
    /// still listing its placeholders has taken none.
    #[test]
    fn ntp_servers_are_listed_up_to_the_slots_the_bmc_has() {
        let list = |entries: &[&str]| entries.iter().map(ToString::to_string).collect::<Vec<_>>();
        let wanted = list(&["10.0.0.1", "10.0.0.2", "10.0.0.3"]);
        let cases = [
            (
                "every server listed",
                list(&["10.0.0.1", "10.0.0.2", "10.0.0.3"]),
                Some(true),
                true,
            ),
            (
                "two slots, first two kept",
                list(&["10.0.0.1", "10.0.0.2"]),
                None,
                true,
            ),
            (
                "factory placeholders",
                list(&["localhost", "127.0.0.1"]),
                Some(false),
                false,
            ),
            (
                "listed but disabled",
                list(&["10.0.0.1", "10.0.0.2"]),
                Some(false),
                false,
            ),
            ("nothing listed", list(&[]), Some(true), false),
        ];
        for (scenario, servers, enabled, listed) in cases {
            assert_eq!(
                Ntp { enabled, servers }.lists(&wanted),
                listed,
                "{scenario}"
            );
        }
    }
}
