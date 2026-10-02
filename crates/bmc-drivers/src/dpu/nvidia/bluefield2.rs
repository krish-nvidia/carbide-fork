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
    bios_host_privilege_level, bios_nic_mode, enable_bmc_rshim, nic_mode_firmware, nic_mode_value,
    no_dpu, require_nic_mode_firmware, restricted_host_privilege, set_bios_host_privilege_level,
};
use crate::resources::patch_bios_attributes;

/// BlueField-2: NIC mode is a BIOS attribute and there is no host rshim control.
pub(crate) struct BlueField2Dpu;

#[async_trait]
impl<B: Bmc> Dpu<B> for BlueField2Dpu {
    /// The mode is unknown on BMC firmware that predates NIC-mode support.
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<DpuStatus, PlatformError> {
        let nic_mode = match nic_mode_firmware(cx).await? {
            Some(firmware) => Some(bios_nic_mode(cx, &firmware).await?),
            None => None,
        };
        Ok(DpuStatus {
            nic_mode,
            host_rshim: None,
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
        patch_bios_attributes(cx, json!({"NicMode": nic_mode_value(mode)}))
            .await
            .map_err(no_dpu)
    }

    /// There is nothing to change, so every requested state is complete.
    async fn set_host_rshim(
        &self,
        _cx: &OpCx<'_, B>,
        _state: RshimState,
    ) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
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
