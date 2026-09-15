/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Dpu, DpuStatus, DriverOutcome, Fetched, HostPrivilegeLevel, NicMode, OpCx, PlatformError,
    RshimState,
};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::oem::nvidia::computer_system::Mode;
use serde::Deserialize;
use serde_json::json;

use crate::dpu::nvidia::support::{
    bios_nic_mode, enable_bmc_rshim, nic_mode_value, no_dpu, oem_action, require_nic_mode_firmware,
    set_host_privilege_level,
};

/// BlueField-3 and later: NIC mode and host RShim are `Oem.Nvidia` actions.
pub(crate) struct BlueField3Dpu;

/// The `Oem/Nvidia` system resource fields nv-redfish does not model.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HostRshim {
    host_rshim: Option<String>,
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
