/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Lockdown, LockdownDesiredState, LockdownScope, LockdownStatus, OpCx,
    PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::dell::{self, ManagerApplyTime};
use crate::lockdown::support::{signal, state_from_signals};
use crate::resources::{attribute_map, bios_update, selected_bios, update_bios_settings};

/// Dell iDRAC lockdown driver.
///
/// Lockdown is the iDRAC system-lockdown switch with `racadm` disabled and the
/// first boot device pinned. The BIOS in-band and UEFI-variable restrictions
/// block IPMI, so they are staged only for an explicit host lockdown and are
/// always lifted on a full unlock; the aggregate status follows the BMC.
pub(crate) struct IdracLockdown;

/// The device lockdown pins first; the XE9680 cannot PXE boot.
async fn first_boot_device<B: Bmc>(cx: &OpCx<'_, B>) -> Result<&'static str, PlatformError> {
    let model = cx.system().await?.raw().model.clone().flatten();
    Ok(match model.as_deref() {
        Some("PowerEdge XE9680") => "UefiHttp",
        _ => "PXE",
    })
}

async fn set_host<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    dell::stage_bios_attributes(
        cx,
        &attribute_map([
            (
                "InBandManageabilityInterface",
                json!(if enabled { "Disabled" } else { "Enabled" }),
            ),
            (
                "UefiVariableAccess",
                json!(if enabled { "Controlled" } else { "Standard" }),
            ),
        ]),
    )
    .await
}

/// Lifts the BIOS restrictions on the next reset; some iDRACs report them
/// read-only, which leaves nothing to lift.
async fn unlock_bios<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let body = bios_update(&attribute_map([
        ("InBandManageabilityInterface", json!("Enabled")),
        ("UefiVariableAccess", json!("Standard")),
    ]))?
    .with_settings_apply_time(dell::on_reset());
    match update_bios_settings(cx, &body).await {
        Ok(response) => dell::job_outcome(cx, response).await,
        Err(error) if dell::is_read_only_attribute(&error) => Ok(DriverOutcome::complete()),
        Err(error) => Err(error),
    }
}

/// System lockdown takes effect at once and rejects later writes, so the
/// other restrictions are staged first.
async fn lock_bmc<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let restrictions = dell::patch_manager_attributes(
        cx,
        json!({
            "Racadm.1.Enable": "Disabled",
            "ServerBoot.1.FirstBootDevice": first_boot_device(cx).await?,
            "ServerBoot.1.BootOnce": "Disabled"
        }),
        Some(ManagerApplyTime::OnReset),
    )
    .await?;
    let lockdown = dell::patch_manager_attributes(
        cx,
        json!({"Lockdown.1.SystemLockdown": "Enabled"}),
        Some(ManagerApplyTime::OnReset),
    )
    .await?;
    Ok(restrictions.merge(lockdown))
}

async fn unlock_bmc<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    dell::patch_manager_attributes(
        cx,
        json!({
            "Lockdown.1.SystemLockdown": "Disabled",
            "Racadm.1.Enable": "Enabled",
            "ServerBoot.1.FirstBootDevice": first_boot_device(cx).await?,
            "ServerBoot.1.BootOnce": "Disabled"
        }),
        Some(ManagerApplyTime::Immediate),
    )
    .await
}

#[async_trait]
impl<B: Bmc> Lockdown<B> for IdracLockdown
where
    B::Error: ActionError,
{
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
            .manager()
            .await?
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
        Ok(LockdownStatus {
            aggregate: bmc,
            message: format!(
                "in_band={in_band:?}, uefi_variable_access={uefi:?}, system_lockdown={system_lockdown:?}, racadm={racadm:?}"
            ),
            host,
            bmc,
        })
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
            LockdownScope::Bmc | LockdownScope::BmcSystemLockdown | LockdownScope::All
                if enabled =>
            {
                lock_bmc(cx).await
            }
            LockdownScope::Bmc | LockdownScope::BmcSystemLockdown => unlock_bmc(cx).await,
            LockdownScope::All => {
                let bmc = unlock_bmc(cx).await?;
                Ok(bmc.merge(unlock_bios(cx).await?))
            }
        }
    }
}
