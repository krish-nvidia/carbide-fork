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
    bios_nic_mode, enable_bmc_rshim, nic_mode_value, no_dpu, require_nic_mode_firmware,
    set_host_privilege_level,
};
use crate::resources::patch_bios_attributes;

/// BlueField-2: no host RShim control; NIC mode is a BIOS attribute.
pub(crate) struct BlueField2Dpu;

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
