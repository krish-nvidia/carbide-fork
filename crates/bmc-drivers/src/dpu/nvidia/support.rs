/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BlueField DPU mechanics shared by the card generations.

use bmc_platform::{DriverOutcome, HostPrivilegeLevel, NicMode, OpCx, PlatformError, RshimState};
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::oem::nvidia::NvidiaComputerSystem;
use nv_redfish::oem::nvidia::computer_system::{HostRshim, Mode};
use serde_json::{Value, json};
use version_compare::Cmp;

use crate::resources::{patch_bios_attributes, selected_bios};

/// NIC mode is readable and switchable through Redfish from this BMC firmware onward.
const NIC_MODE_MINIMUM_FIRMWARE: &str = "BF-23.10-5";
/// Before this BMC firmware, reading BIOS on a DPU in NIC mode fails with a
/// 500 whose body still carries the BIOS attributes.
const NIC_MODE_BIOS_ERROR_FIXED_FIRMWARE: &str = "BF-24.07-14";
/// On this BMC firmware the system `Oem/Nvidia` resource times out on a DPU in NIC mode.
const OEM_EXTENSION_TIMEOUT_FIRMWARE: &str = "BF-24.04-5";

/// BMC 24.10 dropped the spaces from BIOS attribute names.
const HOST_PRIVILEGE_LEVEL: &str = "HostPrivilegeLevel";
const HOST_PRIVILEGE_LEVEL_WITH_SPACES: &str = "Host Privilege Level";

pub(super) fn no_dpu(error: PlatformError) -> PlatformError {
    match error {
        PlatformError::InvalidResponse { .. } => PlatformError::NoDpu,
        other => other,
    }
}

fn compare(version: &str, reference: &str) -> Option<Cmp> {
    version_compare::compare(version, reference).ok()
}

fn parse_mode(value: &str) -> Option<NicMode> {
    match value.replace('"', "").as_str() {
        "NicMode" => Some(NicMode::Nic),
        "DpuMode" => Some(NicMode::Dpu),
        _ => None,
    }
}

pub(super) fn nic_mode_value(mode: NicMode) -> &'static str {
    match mode {
        NicMode::Nic => "NicMode",
        NicMode::Dpu => "DpuMode",
    }
}

/// The version of the firmware inventory entry named `BMC_Firmware`.
async fn bmc_firmware_version<B: Bmc>(cx: &OpCx<'_, B>) -> Result<String, PlatformError> {
    cx.service_root()
        .update_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::NoContent)?
        .firmware_inventories()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::NoContent)?
        .into_iter()
        .map(|inventory| inventory.raw())
        .find(|inventory| inventory.odata_id.to_string().contains("BMC_Firmware"))
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "BlueField BMC firmware inventory was not found".to_string(),
        })?
        .version
        .clone()
        .flatten()
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "BlueField BMC firmware inventory has no version".to_string(),
        })
}

/// The BMC firmware version, or `None` when it predates NIC-mode support.
pub(super) async fn nic_mode_firmware<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<String>, PlatformError> {
    let version = bmc_firmware_version(cx).await?;
    Ok((compare(&version, NIC_MODE_MINIMUM_FIRMWARE) != Some(Cmp::Lt)).then_some(version))
}

/// Fails with `Unsupported` when the BMC firmware predates NIC-mode support.
pub(super) async fn require_nic_mode_firmware<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<(), PlatformError> {
    nic_mode_firmware(cx)
        .await?
        .map(drop)
        .ok_or(PlatformError::Unsupported)
}

/// Whether a failed BIOS read is the NIC-mode 500 whose body reports NIC mode.
fn bios_error_reports_nic_mode(error: &PlatformError) -> bool {
    let PlatformError::Bmc {
        status: 500,
        message,
        ..
    } = error
    else {
        return false;
    };
    serde_json::from_str::<Value>(message)
        .ok()
        .and_then(|body| body.pointer("/Attributes/NicMode").cloned())
        .and_then(|mode| mode.as_str().and_then(parse_mode))
        == Some(NicMode::Nic)
}

/// Reads `NicMode` from the BIOS attributes.
pub(super) async fn bios_nic_mode<B: Bmc>(
    cx: &OpCx<'_, B>,
    firmware: &str,
) -> Result<NicMode, PlatformError> {
    match selected_bios(cx).await {
        Ok(bios) => bios
            .attribute("NicMode")
            .and_then(|value| value.str_value().and_then(parse_mode))
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: "BlueField BIOS does not report NicMode".to_string(),
            }),
        Err(error)
            if compare(firmware, NIC_MODE_BIOS_ERROR_FIXED_FIRMWARE) == Some(Cmp::Lt)
                && bios_error_reports_nic_mode(&error) =>
        {
            Ok(NicMode::Nic)
        }
        Err(error) => Err(no_dpu(error)),
    }
}

/// The system `Oem.Nvidia` resource.
pub(super) async fn system_oem<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<NvidiaComputerSystem<B>, PlatformError> {
    cx.system()
        .map_err(no_dpu)?
        .oem_nvidia()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// The BlueField-3 mode: the system `Oem.Nvidia` mode, falling back to BIOS.
///
/// On the firmware whose OEM resource times out in NIC mode, a BIOS read that
/// fails while reporting NIC mode answers first.
pub(super) async fn system_nic_mode<B: Bmc>(
    cx: &OpCx<'_, B>,
    firmware: &str,
    oem: &NvidiaComputerSystem<B>,
) -> Result<NicMode, PlatformError> {
    if compare(firmware, OEM_EXTENSION_TIMEOUT_FIRMWARE) == Some(Cmp::Eq)
        && selected_bios(cx)
            .await
            .err()
            .is_some_and(|error| bios_error_reports_nic_mode(&error))
    {
        return Ok(NicMode::Nic);
    }
    match oem.mode() {
        Some(Mode::NicMode) => Ok(NicMode::Nic),
        Some(Mode::DpuMode) => Ok(NicMode::Dpu),
        Some(Mode::UnsupportedValue) | None => bios_nic_mode(cx, firmware).await,
    }
}

pub(super) fn host_rshim_state(oem: &NvidiaComputerSystem<impl Bmc>) -> Option<RshimState> {
    match oem.host_rshim()? {
        HostRshim::Enabled => Some(RshimState::Enabled),
        HostRshim::Disabled => Some(RshimState::Disabled),
        HostRshim::UnsupportedValue => None,
    }
}

/// Posts `payload` to the system `Oem.Nvidia` action `action`.
///
/// `NvidiaComputerSystem::set_mode` and `set_host_rshim` require
/// `B::Error: ActionError`, which the production HTTP transport does not
/// implement, so these two actions are still posted to their fixed targets.
pub(super) async fn oem_action<B: Bmc>(
    cx: &OpCx<'_, B>,
    action: &str,
    payload: &Value,
) -> Result<DriverOutcome, PlatformError> {
    let system = cx.system().map_err(no_dpu)?;
    let target = ODataId::from(format!(
        "{}/Oem/Nvidia/Actions/{action}",
        system.raw().odata_id
    ));
    cx.post(&target, payload).await
}

/// Enables the BMC side of rshim through the manager's `Oem.Nvidia` resource.
pub(super) async fn enable_bmc_rshim<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<DriverOutcome, PlatformError> {
    cx.manager()?
        .oem_nvidia()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .set_bmc_rshim_enabled(true)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Stages the host privilege level in BIOS, retrying with the spaced
/// attribute name when the BMC rejects the current one.
pub(super) async fn set_bios_host_privilege_level<B: Bmc>(
    cx: &OpCx<'_, B>,
    level: HostPrivilegeLevel,
) -> Result<DriverOutcome, PlatformError> {
    let value = match level {
        HostPrivilegeLevel::Privileged => "Privileged",
        HostPrivilegeLevel::Restricted => "Restricted",
    };
    match patch_bios_attributes(cx, json!({HOST_PRIVILEGE_LEVEL: value})).await {
        Err(PlatformError::Bmc { message, .. }) if message.contains(HOST_PRIVILEGE_LEVEL) => {
            patch_bios_attributes(cx, json!({HOST_PRIVILEGE_LEVEL_WITH_SPACES: value})).await
        }
        result => result.map_err(no_dpu),
    }
}

#[cfg(test)]
mod tests {
    use carbide_test_support::value_scenarios;

    use super::*;

    #[test]
    fn firmware_gates_compare_like_libredfish() {
        value_scenarios!(run = |version| compare(version, NIC_MODE_MINIMUM_FIRMWARE) == Some(Cmp::Lt);
            "predates NIC-mode support" {
                "BF-23.09-9" => true,
                "BF-23.10-4" => true,
            }
            "supports NIC mode" {
                "BF-23.10-5" => false,
                "BF-24.10-7" => false,
                "BF-26.04-8" => false,
            }
        );
    }

    #[test]
    fn only_a_500_carrying_nic_mode_counts_as_nic_mode() {
        let bios_error = |status, body: &str| PlatformError::Bmc {
            status,
            message_id: None,
            message: body.to_string(),
        };
        let nic = r#"{"Attributes": {"NicMode": "NicMode"}}"#;
        assert!(bios_error_reports_nic_mode(&bios_error(500, nic)));
        assert!(!bios_error_reports_nic_mode(&bios_error(
            500,
            r#"{"Attributes": {"NicMode": "DpuMode"}}"#
        )));
        assert!(!bios_error_reports_nic_mode(&bios_error(400, nic)));
        assert!(!bios_error_reports_nic_mode(&bios_error(500, "not JSON")));
    }
}
