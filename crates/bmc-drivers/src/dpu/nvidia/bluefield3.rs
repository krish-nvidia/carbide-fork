/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Dpu, DpuStatus, DriverOutcome, HostPrivilegeLevel, NicMode, OpCx, PlatformError, RshimState,
};
use nv_redfish::core::Bmc;
use serde_json::json;

use crate::dpu::nvidia::support::{
    bios_host_privilege_level, enable_bmc_rshim, host_rshim_state, nic_mode_firmware,
    nic_mode_value, oem_action, require_nic_mode_firmware, restricted_host_privilege,
    set_bios_host_privilege_level, system_nic_mode, system_oem,
};

/// BlueField-3: mode and host rshim are the system `Oem.Nvidia` properties
/// and actions; host privilege is a BIOS attribute.
pub(crate) struct BlueField3Dpu;

#[async_trait]
impl<B: Bmc> Dpu<B> for BlueField3Dpu {
    /// The mode is unknown on BMC firmware that predates NIC-mode support.
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<DpuStatus, PlatformError> {
        let firmware = nic_mode_firmware(cx).await?;
        let oem = system_oem(cx).await?;
        let nic_mode = match firmware {
            Some(firmware) => Some(system_nic_mode(cx, &firmware, &oem).await?),
            None => None,
        };
        Ok(DpuStatus {
            nic_mode,
            host_rshim: host_rshim_state(&oem),
        })
    }

    async fn set_nic_mode(
        &self,
        cx: &OpCx<'_, B>,
        mode: NicMode,
    ) -> Result<DriverOutcome, PlatformError> {
        require_nic_mode_firmware(cx).await?;
        if mode == NicMode::Nic
            && bios_host_privilege_level(cx).await? == Some(HostPrivilegeLevel::Restricted)
        {
            return Ok(restricted_host_privilege());
        }
        oem_action(cx, "Mode.Set", &json!({"Mode": nic_mode_value(mode)})).await
    }

    async fn set_host_rshim(
        &self,
        cx: &OpCx<'_, B>,
        state: RshimState,
    ) -> Result<DriverOutcome, PlatformError> {
        let host_rshim = match state {
            RshimState::Enabled => "Enabled",
            RshimState::Disabled => "Disabled",
        };
        oem_action(cx, "HostRshim.Set", &json!({"HostRshim": host_rshim})).await
    }

    async fn enable_bmc_rshim(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        enable_bmc_rshim(cx).await
    }

    async fn set_host_privilege_level(
        &self,
        cx: &OpCx<'_, B>,
        level: HostPrivilegeLevel,
    ) -> Result<DriverOutcome, PlatformError> {
        set_bios_host_privilege_level(cx, level).await
    }
}
