/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BlueField DPU mechanics shared by the card generations.

use bmc_platform::{
    DriverOutcome, HostPrivilegeLevel, NicMode, OpCx, PlatformError, Quirk, RshimState,
};
use nv_redfish::computer_system::Bios;
use nv_redfish::core::Bmc;
use nv_redfish::oem::nvidia::NvidiaComputerSystem;
use nv_redfish::oem::nvidia::computer_system::{HostRshim, Mode};
use serde_json::{Value, json};

use crate::resources::{attribute_map, patch_bios_attributes, selected_bios};

/// BMC 24.10 dropped the spaces from BIOS attribute names; see
/// [`Quirk::BlueFieldSpacedBiosAttributeNames`].
const HOST_PRIVILEGE_LEVEL: &str = "HostPrivilegeLevel";
const HOST_PRIVILEGE_LEVEL_WITH_SPACES: &str = "Host Privilege Level";

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

/// Whether a failed BIOS read is the NIC-mode 500 whose body reports NIC mode.
pub(super) fn bios_error_reports_nic_mode(error: &PlatformError) -> bool {
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

fn reported_nic_mode<B: Bmc>(bios: &Bios<B>) -> Result<NicMode, PlatformError> {
    bios.attribute("NicMode")
        .and_then(|value| value.str_value().and_then(parse_mode))
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "BlueField BIOS does not report NicMode".to_string(),
        })
}

/// Reads `NicMode` from the BIOS attributes, reading NIC mode out of the 500
/// that firmware with [`Quirk::BlueFieldNicModeBiosError`] answers with.
pub(super) async fn bios_nic_mode<B: Bmc>(cx: &OpCx<'_, B>) -> Result<NicMode, PlatformError> {
    match selected_bios(cx).await {
        Ok(bios) => reported_nic_mode(&bios),
        Err(error)
            if cx.has_quirk(Quirk::BlueFieldNicModeBiosError)
                && bios_error_reports_nic_mode(&error) =>
        {
            Ok(NicMode::Nic)
        }
        Err(error) => Err(error),
    }
}

/// The system `Oem.Nvidia` resource.
pub(super) async fn system_oem<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<NvidiaComputerSystem<B>, PlatformError> {
    cx.system()
        .await?
        .oem_nvidia()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// The mode the system `Oem.Nvidia` resource reports; BlueField-3 falls
/// back to BIOS without one.
pub(super) fn oem_nic_mode(oem: &NvidiaComputerSystem<impl Bmc>) -> Option<NicMode> {
    match oem.mode()? {
        Mode::NicMode => Some(NicMode::Nic),
        Mode::DpuMode => Some(NicMode::Dpu),
        Mode::UnsupportedValue => None,
    }
}

pub(super) fn host_rshim_state(oem: &NvidiaComputerSystem<impl Bmc>) -> Option<RshimState> {
    match oem.host_rshim()? {
        HostRshim::Enabled => Some(RshimState::Enabled),
        HostRshim::Disabled => Some(RshimState::Disabled),
        HostRshim::UnsupportedValue => None,
    }
}

/// Enables the BMC side of rshim through the manager's `Oem.Nvidia` resource.
pub(super) async fn enable_bmc_rshim<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<DriverOutcome, PlatformError> {
    cx.manager()
        .await?
        .oem_nvidia()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .set_bmc_rshim_enabled(true)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Stages the host privilege level in BIOS under the spelling the firmware
/// uses, retrying with the other spelling when the BMC rejects it.
pub(super) async fn set_bios_host_privilege_level<B: Bmc>(
    cx: &OpCx<'_, B>,
    level: HostPrivilegeLevel,
) -> Result<DriverOutcome, PlatformError> {
    let value = match level {
        HostPrivilegeLevel::Privileged => "Privileged",
        HostPrivilegeLevel::Restricted => "Restricted",
    };
    let (key, other_key) = if cx.has_quirk(Quirk::BlueFieldSpacedBiosAttributeNames) {
        (HOST_PRIVILEGE_LEVEL_WITH_SPACES, HOST_PRIVILEGE_LEVEL)
    } else {
        (HOST_PRIVILEGE_LEVEL, HOST_PRIVILEGE_LEVEL_WITH_SPACES)
    };
    match patch_bios_attributes(cx, &attribute_map([(key, json!(value))])).await {
        Err(PlatformError::Bmc { message, .. }) if message.contains(key) => {
            patch_bios_attributes(cx, &attribute_map([(other_key, json!(value))])).await
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
