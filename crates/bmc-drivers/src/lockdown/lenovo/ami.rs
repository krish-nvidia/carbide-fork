/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Lockdown, LockdownDesiredState, LockdownScope, LockdownStatus, OpCx,
    PlatformError,
};
use nv_redfish::core::{Bmc, EntityTypeRef};
use nv_redfish::oem::ami::config_bmc::{
    LockdownBiosSettingsChangeState, LockdownBiosUpgradeDowngradeState,
    LockoutBiosVariableWriteMode, LockoutHostControlState,
};
use serde_json::json;

use crate::lockdown::support::{signal, state_from_signals, status};

/// Lenovo AMI lockdown driver.
///
/// The OEM `ConfigBMC` object switches host control and BIOS protection
/// together, so only the `All` scope is expressible.
pub(crate) struct LenovoAmiLockdown;

#[async_trait]
impl<B: Bmc> Lockdown<B> for LenovoAmiLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let config = cx
            .manager()?
            .oem_ami_config_bmc()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        let raw = config.raw();
        let signals = [
            signal(
                raw.lockout_host_control,
                LockoutHostControlState::Enable,
                LockoutHostControlState::Disable,
            ),
            signal(
                raw.lockout_bios_variable_write_mode,
                LockoutBiosVariableWriteMode::Enable,
                LockoutBiosVariableWriteMode::Disable,
            ),
            signal(
                raw.lockdown_bios_settings_change,
                LockdownBiosSettingsChangeState::Enable,
                LockdownBiosSettingsChangeState::Disable,
            ),
            signal(
                raw.lockdown_bios_upgrade_downgrade,
                LockdownBiosUpgradeDowngradeState::Enable,
                LockdownBiosUpgradeDowngradeState::Disable,
            ),
        ];
        let state = state_from_signals(&signals);
        Ok(status(
            state,
            state,
            format!(
                "host_control={:?}, bios_variable_write={:?}, bios_settings_change={:?}, bios_upgrade_downgrade={:?}",
                raw.lockout_host_control,
                raw.lockout_bios_variable_write_mode,
                raw.lockdown_bios_settings_change,
                raw.lockdown_bios_upgrade_downgrade
            ),
        ))
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        scope: LockdownScope,
        desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        if scope != LockdownScope::All {
            return Err(PlatformError::Unsupported);
        }
        let config = cx
            .manager()?
            .oem_ami_config_bmc()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        let value = if desired == LockdownDesiredState::Enabled {
            "Enable"
        } else {
            "Disable"
        };
        cx.post(
            config.raw().odata_id(),
            &json!({
                "LockoutHostControl": value,
                "LockoutBiosVariableWriteMode": value,
                "LockdownBiosSettingsChange": value,
                "LockdownBiosUpgradeDowngrade": value
            }),
        )
        .await
    }
}
