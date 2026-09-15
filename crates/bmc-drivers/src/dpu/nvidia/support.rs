/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BlueField DPU mechanics shared by the card generations.

use bmc_platform::{DriverOutcome, HostPrivilegeLevel, NicMode, OpCx, PlatformError};
use nv_redfish::Resource;
use nv_redfish::computer_system::ComputerSystem;
use nv_redfish::core::{Bmc, ODataId};
use serde_json::{Value, json};

use crate::resources::{patch_bios_attributes, selected_bios};

/// NIC mode is switchable through Redfish from this BMC firmware onward.
const NIC_MODE_MINIMUM_FIRMWARE: (u32, u32, u32) = (23, 10, 5);

fn parse_mode(value: &str) -> Option<NicMode> {
    match value.trim_matches('"') {
        "Nic" | "NicMode" => Some(NicMode::Nic),
        "Dpu" | "DpuMode" => Some(NicMode::Dpu),
        _ => None,
    }
}

fn firmware_triplet(version: &str) -> Option<(u32, u32, u32)> {
    let version = version
        .strip_prefix("BF-")
        .or_else(|| version.strip_prefix("bf-"))
        .unwrap_or(version);
    let (year, rest) = version.split_once('.')?;
    let (month, build) = rest.split_once('-')?;
    Some((year.parse().ok()?, month.parse().ok()?, build.parse().ok()?))
}

/// Whether `version` predates `minimum`. Unparseable versions are not treated
/// as old, matching the behavior NICo has relied on so far.
fn firmware_before(version: &str, minimum: (u32, u32, u32)) -> bool {
    firmware_triplet(version).is_some_and(|version| version < minimum)
}

pub(super) fn no_dpu(error: PlatformError) -> PlatformError {
    match error {
        PlatformError::InvalidResponse { .. } => PlatformError::NoDpu,
        other => other,
    }
}

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
        .find(|inventory| inventory.odata_id().to_string().contains("BMC_Firmware"))
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "BlueField BMC firmware inventory was not found".to_string(),
        })?
        .version()
        .map(|version| version.to_string())
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "BlueField BMC firmware inventory has no version".to_string(),
        })
}

pub(super) async fn require_nic_mode_firmware<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<(), PlatformError> {
    if firmware_before(&bmc_firmware_version(cx).await?, NIC_MODE_MINIMUM_FIRMWARE) {
        return Err(PlatformError::Unsupported);
    }
    Ok(())
}

pub(super) async fn bios_nic_mode<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<NicMode>, PlatformError> {
    Ok(selected_bios(cx)
        .await
        .map_err(no_dpu)?
        .attribute("NicMode")
        .and_then(|value| value.str_value().and_then(parse_mode)))
}

pub(super) fn nic_mode_value(mode: NicMode) -> &'static str {
    match mode {
        NicMode::Nic => "NicMode",
        NicMode::Dpu => "DpuMode",
    }
}

pub(super) async fn oem_action<B: Bmc>(
    cx: &OpCx<'_, B>,
    system: &ComputerSystem<B>,
    action: &str,
    payload: &Value,
) -> Result<DriverOutcome, PlatformError> {
    let target = ODataId::from(format!("{}/Oem/Nvidia/Actions/{action}", system.odata_id()));
    cx.post(&target, payload).await
}

/// BMC 24.10 dropped the spaces from this attribute name; use whichever
/// spelling this BIOS reports.
pub(super) async fn set_host_privilege_level<B: Bmc>(
    cx: &OpCx<'_, B>,
    level: HostPrivilegeLevel,
) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await.map_err(no_dpu)?;
    let key = ["HostPrivilegeLevel", "Host Privilege Level"]
        .into_iter()
        .find(|key| bios.attribute(key).is_some())
        .ok_or(PlatformError::Unsupported)?;
    let value = match level {
        HostPrivilegeLevel::Privileged => "Privileged",
        HostPrivilegeLevel::Restricted => "Restricted",
    };
    patch_bios_attributes(cx, json!({key: value})).await
}

pub(super) async fn enable_bmc_rshim<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<DriverOutcome, PlatformError> {
    let target = ODataId::from(format!("{}/Oem/Nvidia", cx.manager()?.odata_id()));
    cx.patch_id(
        &target,
        None,
        &json!({"BmcRShim": {"BmcRShimEnabled": true}}),
    )
    .await
    .map(DriverOutcome::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bluefield_firmware_names() {
        assert_eq!(firmware_triplet("BF-24.10-7"), Some((24, 10, 7)));
        assert_eq!(firmware_triplet("23.10-5"), Some((23, 10, 5)));
        assert_eq!(firmware_triplet("not-a-version"), None);
        assert!(!firmware_before("BF-24.10-0", (24, 10, 0)));
        assert!(firmware_before("BF-23.09-9", NIC_MODE_MINIMUM_FIRMWARE));
        assert!(!firmware_before("unknown", NIC_MODE_MINIMUM_FIRMWARE));
    }

    #[test]
    fn parses_nic_mode_spellings() {
        assert_eq!(parse_mode("NicMode"), Some(NicMode::Nic));
        assert_eq!(parse_mode("\"DpuMode\""), Some(NicMode::Dpu));
        assert_eq!(parse_mode("unknown"), None);
    }
}
