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

//! HPE iLO platform plugin (ProLiant servers managed by iLO 5/6/7).
//!
//! The plugin works purely through the [`RedfishOps`] façade -- Redfish paths
//! and JSON -- and never touches the transport. That keeps it small and lets a
//! generated plugin mirror an operator's curl probe one-to-one.
//!
//! ## Handling iLO version divergence
//!
//! iLO firmware diverges across generations, handled here with the more robust
//! of two strategies:
//!
//! 1. **BIOS attribute renames** (e.g. the virtualization toggle is
//!    `IntelProcVtd`, `ProcAmdIoVt`, or `ProcVirtualization` by CPU vendor /
//!    generation). The plugin **probes the BIOS attribute set** and uses
//!    whichever candidate key is present ([`resolve_bios_key`]) instead of
//!    branching on a firmware string.
//! 2. **OEM features gated by firmware** (e.g. KCS lockdown via the manager
//!    `NetworkProtocol` OEM section). The plugin **probes for the field** and
//!    acts only when it is present, rather than comparing firmware versions.
//!
//! Probing-over-sniffing keeps the plugin correct on firmware we have not seen.

use async_trait::async_trait;
use carbide_redfish_platform_api::RedfishError;
use carbide_redfish_platform_api::model::{
    BmcAccountPolicyRequest, BmcDeleteUserRequest, BmcPasswordRequest, BmcResetKind, BmcStatus,
    BmcUserRequest, BootOrderRequest, BootOrderStatus, FirmwareComponent, FirmwareInventory,
    FirmwareUpdateRequest, JobHandle, LockdownStatus, MachineSetupRequest, MachineSetupStatus,
    MatchSpecificity, PlatformIdentity, PlatformMetadata, PluginId, PowerAction, PowerState,
    SecureBootStatus, SupportLevel,
};
use carbide_redfish_platform_api::ops::PlatformExecutionContext;
use carbide_redfish_platform_api::plugin::{
    BmcAccountCap, BmcResetCap, BootOrderCap, FirmwareCap, HostPowerCap, LockdownCap,
    MachineSetupCap, PlatformPlugin, SecureBootCap,
};
use serde_json::{Value, json};

use crate::providers;

/// HPE iLO plugin.
pub struct HpeIloPlugin {
    metadata: PlatformMetadata,
    power: IloPower,
    bmc_reset: IloBmcReset,
    machine_setup: IloMachineSetup,
    boot_order: IloBootOrder,
    secure_boot: IloSecureBoot,
    lockdown: IloLockdown,
    accounts: IloAccounts,
    firmware: IloFirmware,
}

impl HpeIloPlugin {
    /// Construct the HPE iLO plugin.
    pub fn new() -> Self {
        Self {
            metadata: PlatformMetadata {
                id: PluginId("nico.redfish.hpe.ilo".to_string()),
                vendor: "HPE",
                models: &["ProLiant"],
                plugin_version: env!("CARGO_PKG_VERSION"),
                support_level: SupportLevel::Candidate,
            },
            power: IloPower,
            bmc_reset: IloBmcReset,
            machine_setup: IloMachineSetup,
            boot_order: IloBootOrder,
            secure_boot: IloSecureBoot,
            lockdown: IloLockdown,
            accounts: IloAccounts,
            firmware: IloFirmware,
        }
    }
}

impl Default for HpeIloPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformPlugin for HpeIloPlugin {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    fn detect(&self, identity: &PlatformIdentity) -> Option<MatchSpecificity> {
        let is_hpe = identity
            .service_root_vendor
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("hpe"));
        is_hpe.then_some(MatchSpecificity::Vendor)
    }

    fn power(&self) -> Option<&dyn HostPowerCap> {
        Some(&self.power)
    }
    fn bmc_reset(&self) -> Option<&dyn BmcResetCap> {
        Some(&self.bmc_reset)
    }
    fn machine_setup(&self) -> Option<&dyn MachineSetupCap> {
        Some(&self.machine_setup)
    }
    fn boot_order(&self) -> Option<&dyn BootOrderCap> {
        Some(&self.boot_order)
    }
    fn secure_boot(&self) -> Option<&dyn SecureBootCap> {
        Some(&self.secure_boot)
    }
    fn lockdown(&self) -> Option<&dyn LockdownCap> {
        Some(&self.lockdown)
    }
    fn accounts(&self) -> Option<&dyn BmcAccountCap> {
        Some(&self.accounts)
    }
    fn firmware(&self) -> Option<&dyn FirmwareCap> {
        Some(&self.firmware)
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (all JSON over the RedfishOps façade)
// ---------------------------------------------------------------------------

const VIRTUALIZATION_KEYS: &[&str] = &["IntelProcVtd", "ProcAmdIoVt", "ProcVirtualization"];

/// The BIOS serial-console attributes NICo sets, shared by apply and status.
const SERIAL_CONSOLE_ATTRS: &[(&str, &str)] = &[
    ("EmbeddedSerialPort", "Com2Irq3"),
    ("EmsConsole", "Virtual"),
    ("SerialConsoleBaudRate", "BaudRate115200"),
    ("SerialConsoleEmulation", "Vt100Plus"),
    ("SerialConsolePort", "Virtual"),
    ("UefiSerialDebugLevel", "ErrorsOnly"),
    ("VirtualSerialPort", "Com1Irq4"),
];

/// PATCH a set of BIOS attributes onto the staged settings object.
async fn patch_bios(
    ctx: &PlatformExecutionContext<'_>,
    system_path: &str,
    attributes: Value,
) -> Result<(), RedfishError> {
    ctx.ops()
        .patch(
            &format!("{system_path}/Bios/settings"),
            json!({ "Attributes": attributes }),
        )
        .await
}

/// Read the system's current BIOS attribute object.
async fn bios_attributes(
    ctx: &PlatformExecutionContext<'_>,
    system_path: &str,
) -> Result<Value, RedfishError> {
    let bios = ctx.ops().get(&format!("{system_path}/Bios")).await?;
    Ok(bios.get("Attributes").cloned().unwrap_or(Value::Null))
}

/// Pick the first candidate BIOS attribute key present on this platform.
fn resolve_bios_key(attributes: &Value, candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|key| attributes.get(key).is_some())
}

/// True if a BIOS string attribute equals the expected value.
fn attr_is(attributes: &Value, key: &str, expected: &str) -> bool {
    attributes.get(key).and_then(|v| v.as_str()) == Some(expected)
}

// ---------------------------------------------------------------------------
// Power
// ---------------------------------------------------------------------------

/// HPE power control.
pub struct IloPower;

#[async_trait]
impl HostPowerCap for IloPower {
    async fn power_state(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<PowerState, RedfishError> {
        providers::standard_power_state(ctx).await
    }

    async fn set_power(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        action: PowerAction,
    ) -> Result<(), RedfishError> {
        match action {
            // iLO performs a warm reset for a force restart via GracefulRestart.
            PowerAction::ForceRestart => {
                providers::standard_set_power(ctx, "GracefulRestart").await
            }
            // AC power cycle: ensure the host is off, then the OEM AuxCycle.
            // NOTE: ForceOff is asynchronous; iLO accepts AuxCycle regardless of
            // the current power state, so we do not block on the host settling.
            PowerAction::AcPowerCycle => {
                if providers::standard_power_state(ctx).await? != PowerState::Off {
                    providers::standard_set_power(ctx, "ForceOff").await?;
                }
                let system_path = ctx.ops().system_path().await?;
                let target =
                    format!("{system_path}/Actions/Oem/Hpe/HpeComputerSystemExt.SystemReset");
                ctx.ops()
                    .post_action(&target, json!({ "ResetType": "AuxCycle" }))
                    .await?;
                Ok(())
            }
            // Everything else maps to the standard ComputerSystem.Reset types.
            other => {
                let reset_type = providers::reset_type(other)
                    .ok_or_else(|| RedfishError::not_supported("unsupported power action"))?;
                providers::standard_set_power(ctx, reset_type).await
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BMC reset
// ---------------------------------------------------------------------------

/// HPE manager reset.
pub struct IloBmcReset;

#[async_trait]
impl BmcResetCap for IloBmcReset {
    async fn bmc_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<BmcStatus, RedfishError> {
        Ok(BmcStatus {
            ready: true,
            firmware_version: providers::standard_manager_firmware(ctx).await?,
        })
    }

    async fn reset_bmc(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        kind: BmcResetKind,
    ) -> Result<(), RedfishError> {
        match kind {
            BmcResetKind::GracefulRestart => {
                providers::standard_reset_bmc(ctx, "GracefulRestart").await
            }
            BmcResetKind::ForceRestart => providers::standard_reset_bmc(ctx, "ForceRestart").await,
            BmcResetKind::ResetToDefaults => {
                let manager_path = ctx.ops().manager_path().await?;
                let target = format!("{manager_path}/Actions/Manager.ResetToDefaults");
                ctx.ops()
                    .post_action(&target, json!({ "ResetType": "ResetAll" }))
                    .await?;
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Machine setup
// ---------------------------------------------------------------------------

/// HPE machine/BIOS setup.
pub struct IloMachineSetup;

#[async_trait]
impl MachineSetupCap for IloMachineSetup {
    async fn apply_machine_setup(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: MachineSetupRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let system_path = ctx.ops().system_path().await?;

        if req.enable_network_boot {
            // Enable UEFI HTTP/PXE network boot so NICo can network-boot the host.
            patch_bios(
                ctx,
                &system_path,
                json!({ "Dhcpv4": "Enabled", "HttpSupport": "Auto" }),
            )
            .await?;
        }

        if req.enable_virtualization {
            // Resolve the virtualization attribute name for this CPU vendor /
            // iLO generation, then enable it.
            let attributes = bios_attributes(ctx, &system_path).await?;
            let virt_key = resolve_bios_key(&attributes, VIRTUALIZATION_KEYS).ok_or_else(|| {
                RedfishError::MissingKey {
                    key: VIRTUALIZATION_KEYS.join(" | "),
                    url: format!("{system_path}/Bios"),
                }
            })?;
            patch_bios(ctx, &system_path, json!({ virt_key: "Enabled" })).await?;
        }

        if req.enable_serial_console {
            let serial: serde_json::Map<String, Value> = SERIAL_CONSOLE_ATTRS
                .iter()
                .map(|(key, value)| ((*key).to_string(), Value::String((*value).to_string())))
                .collect();
            patch_bios(ctx, &system_path, Value::Object(serial)).await?;
        }

        // iLO applies staged BIOS attributes on the next reboot; no job id.
        Ok(None)
    }

    async fn machine_setup_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<MachineSetupStatus, RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        let attributes = bios_attributes(ctx, &system_path).await?;

        // Network boot: DHCP-driven UEFI network stack enabled with HTTP boot.
        let network_boot = attr_is(&attributes, "Dhcpv4", "Enabled")
            && attributes
                .get("HttpSupport")
                .and_then(|v| v.as_str())
                .is_some();

        // Virtualization: whichever attribute this CPU/iLO generation exposes.
        let virtualization = resolve_bios_key(&attributes, VIRTUALIZATION_KEYS)
            .is_some_and(|key| attr_is(&attributes, key, "Enabled"));

        // Serial console: every attribute matches the configured value.
        let serial_console = SERIAL_CONSOLE_ATTRS
            .iter()
            .all(|(key, value)| attr_is(&attributes, key, value));

        let applied = network_boot && virtualization && serial_console;
        Ok(MachineSetupStatus {
            applied,
            // Staged BIOS attributes apply on the next host reboot.
            reboot_required: !applied,
        })
    }

    async fn set_uefi_password(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        password: String,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        patch_bios(ctx, &system_path, json!({ "AdminPassword": password })).await?;
        Ok(None)
    }

    async fn clear_nvram(&self, ctx: &PlatformExecutionContext<'_>) -> Result<(), RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        patch_bios(
            ctx,
            &system_path,
            json!({ "RestoreManufacturingDefaults": "Yes" }),
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Boot order
// ---------------------------------------------------------------------------

/// HPE boot order via the OEM persistent boot config order.
pub struct IloBootOrder;

async fn read_boot_order(
    ctx: &PlatformExecutionContext<'_>,
    system_path: &str,
) -> Result<Vec<String>, RedfishError> {
    let boot = ctx
        .ops()
        .get(&format!("{system_path}/Bios/oem/hpe/boot"))
        .await?;
    Ok(boot
        .get("PersistentBootConfigOrder")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default())
}

#[async_trait]
impl BootOrderCap for IloBootOrder {
    async fn set_dpu_first_boot(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        _req: BootOrderRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        let order = read_boot_order(ctx, &system_path).await?;
        if order.is_empty() {
            return Err(RedfishError::MissingBootOption(
                "no persistent boot config order".to_string(),
            ));
        }
        // Move network ("nic.") entries to the front.
        let mut reordered: Vec<String> = Vec::with_capacity(order.len());
        for entry in order {
            if entry.to_ascii_lowercase().contains("nic.") {
                reordered.insert(0, entry);
            } else {
                reordered.push(entry);
            }
        }
        ctx.ops()
            .patch(
                &format!("{system_path}/Bios/oem/hpe/boot/settings"),
                json!({ "PersistentBootConfigOrder": reordered }),
            )
            .await?;
        Ok(None)
    }

    async fn boot_order_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<BootOrderStatus, RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        let order = read_boot_order(ctx, &system_path).await?;
        let dpu_first = order
            .first()
            .is_some_and(|first| first.to_ascii_lowercase().contains("nic."));
        Ok(BootOrderStatus { dpu_first })
    }

    async fn set_infinite_boot(
        &self,
        _ctx: &PlatformExecutionContext<'_>,
        _enabled: bool,
    ) -> Result<Option<JobHandle>, RedfishError> {
        Err(RedfishError::not_supported(
            "iLO does not expose an infinite-boot control",
        ))
    }
}

// ---------------------------------------------------------------------------
// Secure boot
// ---------------------------------------------------------------------------

/// HPE secure boot (standard SecureBoot resource).
pub struct IloSecureBoot;

#[async_trait]
impl SecureBootCap for IloSecureBoot {
    async fn secure_boot_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<SecureBootStatus, RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        let sb = ctx.ops().get(&format!("{system_path}/SecureBoot")).await?;
        Ok(SecureBootStatus {
            enabled: sb
                .get("SecureBootEnable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    async fn set_secure_boot(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        ctx.ops()
            .patch(
                &format!("{system_path}/SecureBoot"),
                json!({ "SecureBootEnable": enabled }),
            )
            .await
    }

    async fn add_certificate(
        &self,
        _ctx: &PlatformExecutionContext<'_>,
        _certificate: Vec<u8>,
    ) -> Result<Option<JobHandle>, RedfishError> {
        Err(RedfishError::not_supported(
            "iLO secure-boot certificate upload is not implemented",
        ))
    }
}

// ---------------------------------------------------------------------------
// Lockdown
// ---------------------------------------------------------------------------

/// HPE lockdown across host (USB boot), BMC (virtual NIC), and KCS.
pub struct IloLockdown;

impl IloLockdown {
    /// Whether the manager `NetworkProtocol` resource exposes the OEM KCS field
    /// (capability probe -- avoids depending on a firmware-version threshold).
    async fn kcs_supported(ctx: &PlatformExecutionContext<'_>, netproto_path: &str) -> bool {
        match ctx.ops().get(netproto_path).await {
            Ok(np) => np
                .pointer("/Oem/Hpe/KcsEnabled")
                .is_some_and(|v| !v.is_null()),
            Err(_) => false,
        }
    }
}

#[async_trait]
impl LockdownCap for IloLockdown {
    async fn lockdown_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<LockdownStatus, RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        let attributes = bios_attributes(ctx, &system_path).await?;
        let host_enabled = attributes.get("UsbBoot").and_then(|v| v.as_str()) == Some("Disabled");

        let manager_path = ctx.ops().manager_path().await?;
        let manager = ctx.ops().get(&manager_path).await?;
        let virtual_nic_enabled = manager
            .pointer("/Oem/Hpe/VirtualNICEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(LockdownStatus {
            host_enabled,
            bmc_enabled: !virtual_nic_enabled,
        })
    }

    async fn set_host_lockdown(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError> {
        let system_path = ctx.ops().system_path().await?;
        // Locked => USB boot disabled.
        let usb_boot = if enabled { "Disabled" } else { "Enabled" };
        patch_bios(ctx, &system_path, json!({ "UsbBoot": usb_boot })).await
    }

    async fn set_bmc_lockdown(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError> {
        let manager_path = ctx.ops().manager_path().await?;

        let netproto_path = format!("{manager_path}/NetworkProtocol");
        if Self::kcs_supported(ctx, &netproto_path).await {
            // Locked => KCS disabled.
            ctx.ops()
                .patch(
                    &netproto_path,
                    json!({ "Oem": { "Hpe": { "KcsEnabled": !enabled } } }),
                )
                .await?;
        }

        // Locked => virtual NIC disabled.
        ctx.ops()
            .patch(
                &manager_path,
                json!({ "Oem": { "Hpe": { "VirtualNICEnabled": !enabled } } }),
            )
            .await
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// HPE BMC accounts (standard AccountService).
pub struct IloAccounts;

const ACCOUNTS_PATH: &str = "/redfish/v1/AccountService/Accounts";
const ACCOUNT_SERVICE_PATH: &str = "/redfish/v1/AccountService";

async fn find_account(
    ctx: &PlatformExecutionContext<'_>,
    username: &str,
) -> Result<Option<String>, RedfishError> {
    let collection = ctx.ops().get(ACCOUNTS_PATH).await?;
    let Some(members) = collection.get("Members").and_then(|m| m.as_array()) else {
        return Ok(None);
    };
    for member in members {
        let Some(path) = member.get("@odata.id").and_then(|v| v.as_str()) else {
            continue;
        };
        let account = ctx.ops().get(path).await?;
        if account.get("UserName").and_then(|v| v.as_str()) == Some(username) {
            return Ok(Some(path.to_string()));
        }
    }
    Ok(None)
}

#[async_trait]
impl BmcAccountCap for IloAccounts {
    async fn ensure_user(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcUserRequest,
    ) -> Result<(), RedfishError> {
        let role = req.role_id.unwrap_or_else(|| "Administrator".to_string());
        if let Some(path) = find_account(ctx, &req.username).await? {
            ctx.ops()
                .patch(
                    &path,
                    json!({ "Password": req.password, "RoleId": role, "Enabled": true }),
                )
                .await
        } else {
            ctx.ops()
                .create(
                    ACCOUNTS_PATH,
                    json!({
                        "UserName": req.username,
                        "Password": req.password,
                        "RoleId": role,
                        "Enabled": true
                    }),
                )
                .await?;
            Ok(())
        }
    }

    async fn delete_user(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcDeleteUserRequest,
    ) -> Result<(), RedfishError> {
        let path = find_account(ctx, &req.username)
            .await?
            .ok_or_else(|| RedfishError::UserNotFound(req.username.clone()))?;
        ctx.ops().delete(&path).await
    }

    async fn change_password(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcPasswordRequest,
    ) -> Result<(), RedfishError> {
        let path = find_account(ctx, &req.username)
            .await?
            .ok_or_else(|| RedfishError::UserNotFound(req.username.clone()))?;
        ctx.ops()
            .patch(&path, json!({ "Password": req.new_password }))
            .await
    }

    async fn set_account_policy(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcAccountPolicyRequest,
    ) -> Result<(), RedfishError> {
        let mut body = serde_json::Map::new();
        if let Some(min_len) = req.min_password_length {
            body.insert("MinPasswordLength".to_string(), json!(min_len));
        }
        if let Some(max_failed) = req.max_failed_logins {
            body.insert("AccountLockoutThreshold".to_string(), json!(max_failed));
        }
        if body.is_empty() {
            return Ok(());
        }
        ctx.ops()
            .patch(ACCOUNT_SERVICE_PATH, Value::Object(body))
            .await
    }
}

// ---------------------------------------------------------------------------
// Firmware
// ---------------------------------------------------------------------------

/// HPE firmware update + inventory (standard UpdateService).
pub struct IloFirmware;

#[async_trait]
impl FirmwareCap for IloFirmware {
    async fn start_update(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: FirmwareUpdateRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let mut body = json!({ "ImageURI": req.image_uri });
        if !req.targets.is_empty() {
            body["Targets"] = json!(req.targets);
        }
        ctx.ops()
            .post_action(
                "/redfish/v1/UpdateService/Actions/UpdateService.SimpleUpdate",
                body,
            )
            .await?;
        Ok(None)
    }

    async fn firmware_inventory(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<FirmwareInventory, RedfishError> {
        let collection = ctx
            .ops()
            .get("/redfish/v1/UpdateService/FirmwareInventory")
            .await?;
        let mut components = Vec::new();
        if let Some(members) = collection.get("Members").and_then(|m| m.as_array()) {
            for member in members {
                let Some(path) = member.get("@odata.id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let inv = ctx.ops().get(path).await?;
                components.push(FirmwareComponent {
                    id: inv
                        .get("Id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(path)
                        .to_string(),
                    version: inv
                        .get("Version")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                });
            }
        }
        Ok(FirmwareInventory { components })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(vendor: Option<&str>) -> PlatformIdentity {
        PlatformIdentity {
            service_root_vendor: vendor.map(str::to_string),
            ..PlatformIdentity::default()
        }
    }

    #[test]
    fn detects_hpe_vendor() {
        let plugin = HpeIloPlugin::new();
        assert_eq!(
            plugin.detect(&identity(Some("HPE"))),
            Some(MatchSpecificity::Vendor)
        );
        assert_eq!(
            plugin.detect(&identity(Some("hpe"))),
            Some(MatchSpecificity::Vendor)
        );
        assert_eq!(plugin.detect(&identity(Some("Dell"))), None);
        assert_eq!(plugin.detect(&identity(None)), None);
    }

    #[test]
    fn implements_full_host_capability_set() {
        let p = HpeIloPlugin::new();
        assert!(p.power().is_some());
        assert!(p.bmc_reset().is_some());
        assert!(p.machine_setup().is_some());
        assert!(p.boot_order().is_some());
        assert!(p.secure_boot().is_some());
        assert!(p.lockdown().is_some());
        assert!(p.accounts().is_some());
        assert!(p.firmware().is_some());
        assert!(p.dpu().is_none());
        assert!(p.storage().is_none());
    }
}
