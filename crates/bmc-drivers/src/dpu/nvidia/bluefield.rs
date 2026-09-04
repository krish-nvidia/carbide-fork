/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! NVIDIA BlueField capability drivers.

use async_trait::async_trait;
use bmc_platform::{
    Dpu, DpuStatus, DriverOutcome, Fetched, HostPrivilegeLevel, NicMode, OpCx, PlatformError,
    RshimState,
};
use nv_redfish::Resource;
use nv_redfish::computer_system::ComputerSystem;
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::oem::nvidia::computer_system::Mode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::resources::{patch_bios_attributes, selected_bios};

/// BlueField-3 and later: NIC mode and host RShim are `Oem.Nvidia` actions.
pub(crate) struct BlueField3Dpu;

/// BlueField-2: no host RShim control; NIC mode is a BIOS attribute.
pub(crate) struct BlueField2Dpu;

/// NIC mode is switchable through Redfish from this BMC firmware onward.
const NIC_MODE_MINIMUM_FIRMWARE: (u32, u32, u32) = (23, 10, 5);

/// The `Oem/Nvidia` system resource fields nv-redfish does not model.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HostRshim {
    host_rshim: Option<String>,
}

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

fn no_dpu(error: PlatformError) -> PlatformError {
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

async fn require_nic_mode_firmware<B: Bmc>(cx: &OpCx<'_, B>) -> Result<(), PlatformError> {
    if firmware_before(&bmc_firmware_version(cx).await?, NIC_MODE_MINIMUM_FIRMWARE) {
        return Err(PlatformError::Unsupported);
    }
    Ok(())
}

async fn bios_nic_mode<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Option<NicMode>, PlatformError> {
    Ok(selected_bios(cx)
        .await
        .map_err(no_dpu)?
        .attribute("NicMode")
        .and_then(|value| value.str_value().and_then(parse_mode)))
}

fn nic_mode_value(mode: NicMode) -> &'static str {
    match mode {
        NicMode::Nic => "NicMode",
        NicMode::Dpu => "DpuMode",
    }
}

async fn oem_action<B: Bmc>(
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
async fn set_host_privilege_level<B: Bmc>(
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

async fn enable_bmc_rshim<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let target = ODataId::from(format!("{}/Oem/Nvidia", cx.manager()?.odata_id()));
    cx.patch_id(
        &target,
        None,
        &json!({"BmcRShim": {"BmcRShimEnabled": true}}),
    )
    .await
    .map(DriverOutcome::from)
}

#[async_trait]
impl<B: Bmc> Dpu<B> for BlueField3Dpu {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<DpuStatus, PlatformError> {
        let system = cx.system().map_err(no_dpu)?;
        let oem = system
            .oem_nvidia()
            .await
            .map_err(|error| cx.map_redfish_error(error))?;
        let nic_mode = match oem.as_ref().and_then(|oem| oem.mode()) {
            Some(Mode::NicMode) => Some(NicMode::Nic),
            Some(Mode::DpuMode) => Some(NicMode::Dpu),
            Some(Mode::UnsupportedValue) | None => bios_nic_mode(cx).await?,
        };
        let target = ODataId::from(format!("{}/Oem/Nvidia", system.odata_id()));
        let host_rshim = cx
            .bmc()
            .get::<Fetched<HostRshim>>(&target)
            .await
            .map_err(|error| cx.map_bmc_error(error))?
            .host_rshim
            .as_deref()
            .and_then(|value| match value {
                "Enabled" => Some(RshimState::Enabled),
                "Disabled" => Some(RshimState::Disabled),
                _ => None,
            });
        Ok(DpuStatus {
            nic_mode,
            host_rshim,
        })
    }

    async fn set_nic_mode(
        &self,
        cx: &OpCx<'_, B>,
        mode: NicMode,
    ) -> Result<DriverOutcome, PlatformError> {
        require_nic_mode_firmware(cx).await?;
        let system = cx.system().map_err(no_dpu)?;
        oem_action(
            cx,
            system,
            "Mode.Set",
            &json!({"Mode": nic_mode_value(mode)}),
        )
        .await
    }

    async fn set_host_rshim(
        &self,
        cx: &OpCx<'_, B>,
        state: RshimState,
    ) -> Result<DriverOutcome, PlatformError> {
        let system = cx.system().map_err(no_dpu)?;
        let host_rshim = match state {
            RshimState::Enabled => "Enabled",
            RshimState::Disabled => "Disabled",
        };
        oem_action(
            cx,
            system,
            "HostRshim.Set",
            &json!({"HostRshim": host_rshim}),
        )
        .await
    }

    async fn enable_bmc_rshim(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        enable_bmc_rshim(cx).await
    }

    async fn set_host_privilege_level(
        &self,
        cx: &OpCx<'_, B>,
        level: HostPrivilegeLevel,
    ) -> Result<DriverOutcome, PlatformError> {
        set_host_privilege_level(cx, level).await
    }
}

#[async_trait]
impl<B: Bmc> Dpu<B> for BlueField2Dpu {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<DpuStatus, PlatformError> {
        Ok(DpuStatus {
            nic_mode: bios_nic_mode(cx).await?,
            host_rshim: None,
        })
    }

    async fn set_nic_mode(
        &self,
        cx: &OpCx<'_, B>,
        mode: NicMode,
    ) -> Result<DriverOutcome, PlatformError> {
        require_nic_mode_firmware(cx).await?;
        patch_bios_attributes(cx, json!({"NicMode": nic_mode_value(mode)}))
            .await
            .map_err(no_dpu)
    }

    async fn set_host_rshim(
        &self,
        _cx: &OpCx<'_, B>,
        state: RshimState,
    ) -> Result<DriverOutcome, PlatformError> {
        // BlueField-2 exposes no host RShim control, so it is never enabled.
        match state {
            RshimState::Disabled => Ok(DriverOutcome::complete()),
            RshimState::Enabled => Err(PlatformError::Unsupported),
        }
    }

    async fn enable_bmc_rshim(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        enable_bmc_rshim(cx).await
    }

    async fn set_host_privilege_level(
        &self,
        cx: &OpCx<'_, B>,
        level: HostPrivilegeLevel,
    ) -> Result<DriverOutcome, PlatformError> {
        set_host_privilege_level(cx, level).await
    }
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
