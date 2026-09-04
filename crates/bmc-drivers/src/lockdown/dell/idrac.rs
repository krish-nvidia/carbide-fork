/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Lockdown, LockdownDesiredState, LockdownScope, LockdownStatus, OpCx,
    PlatformError,
};
use nv_redfish::core::Bmc;
use serde_json::json;

use crate::dell;
use crate::lockdown::support::{signal, state_from_signals, status};
use crate::resources::selected_bios;

/// Dell iDRAC lockdown driver.
///
/// Host lockdown is two BIOS attributes staged as a job; BMC lockdown is the
/// iDRAC system-lockdown attribute plus, unless only that switch is requested,
/// disabling `racadm`.
pub(crate) struct IdracLockdown;

async fn set_host<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    dell::stage_bios_attributes(
        cx,
        json!({
            "InBandManageabilityInterface": if enabled { "Disabled" } else { "Enabled" },
            "UefiVariableAccess": if enabled { "Controlled" } else { "Standard" }
        }),
    )
    .await
}

async fn set_bmc<B: Bmc>(
    cx: &OpCx<'_, B>,
    enabled: bool,
    system_lockdown_only: bool,
) -> Result<DriverOutcome, PlatformError> {
    let mut attributes = json!({
        "Lockdown.1.SystemLockdown": if enabled { "Enabled" } else { "Disabled" }
    });
    if !system_lockdown_only {
        attributes["Racadm.1.Enable"] = (if enabled { "Disabled" } else { "Enabled" }).into();
    }
    dell::patch_manager_attributes(cx, attributes).await
}

#[async_trait]
impl<B: Bmc> Lockdown<B> for IdracLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let bios = selected_bios(cx).await?;
        let in_band = bios
            .attribute("InBandManageabilityInterface")
            .and_then(|value| value.str_value().map(str::to_owned));
        let uefi = bios
            .attribute("UefiVariableAccess")
            .and_then(|value| value.str_value().map(str::to_owned));
        let host = state_from_signals(&[
            signal(in_band.as_deref(), "Disabled", "Enabled"),
            signal(uefi.as_deref(), "Controlled", "Standard"),
        ]);

        let attrs = cx
            .manager()?
            .oem_dell_attributes()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        let system_lockdown = attrs
            .attribute("Lockdown.1.SystemLockdown")
            .and_then(|value| value.str_value().map(str::to_owned));
        let racadm = attrs
            .attribute("Racadm.1.Enable")
            .and_then(|value| value.str_value().map(str::to_owned));
        let bmc = state_from_signals(&[
            signal(system_lockdown.as_deref(), "Enabled", "Disabled"),
            signal(racadm.as_deref(), "Disabled", "Enabled"),
        ]);
        Ok(status(
            host,
            bmc,
            format!(
                "in_band={in_band:?}, uefi_variable_access={uefi:?}, system_lockdown={system_lockdown:?}, racadm={racadm:?}"
            ),
        ))
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        scope: LockdownScope,
        desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        let enabled = desired == LockdownDesiredState::Enabled;
        match scope {
            LockdownScope::Host => set_host(cx, enabled).await,
            LockdownScope::Bmc => set_bmc(cx, enabled, false).await,
            LockdownScope::BmcSystemLockdown => set_bmc(cx, enabled, true).await,
            // The BMC must be unlocked before the host so its writes are accepted.
            LockdownScope::All if enabled => {
                let host = set_host(cx, enabled).await?;
                Ok(host.merge(set_bmc(cx, enabled, false).await?))
            }
            LockdownScope::All => {
                let bmc = set_bmc(cx, enabled, false).await?;
                Ok(bmc.merge(set_host(cx, enabled).await?))
            }
        }
    }
}
