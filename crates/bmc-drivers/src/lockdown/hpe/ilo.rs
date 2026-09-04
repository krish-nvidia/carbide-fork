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

use crate::lockdown::support::{signal, state_from_signals, status};
use crate::resources::{patch_bios_attributes, selected_bios};

/// HPE iLO lockdown driver: USB boot for the host, the virtual NIC for the BMC.
pub(crate) struct IloLockdown;

async fn set_host<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    patch_bios_attributes(
        cx,
        json!({"UsbBoot": if enabled { "Disabled" } else { "Enabled" }}),
    )
    .await
}

async fn set_bmc<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    let manager = cx.manager()?;
    let raw = manager.raw();
    cx.patch(
        raw.as_ref(),
        &json!({"Oem": {"Hpe": {"VirtualNICEnabled": !enabled}}}),
    )
    .await
}

#[async_trait]
impl<B: Bmc> Lockdown<B> for IloLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let usb_boot = selected_bios(cx)
            .await?
            .attribute("UsbBoot")
            .and_then(|value| value.str_value().map(str::to_owned));
        let host = state_from_signals(&[signal(usb_boot.as_deref(), "Disabled", "Enabled")]);

        let virtual_nic = cx
            .manager()?
            .oem_hpe()
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .virtual_nic_enabled();
        let bmc = state_from_signals(&[signal(virtual_nic, false, true)]);
        Ok(status(
            host,
            bmc,
            format!("usb_boot={usb_boot:?}, virtual_nic_enabled={virtual_nic:?}"),
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
            LockdownScope::Bmc | LockdownScope::BmcSystemLockdown => set_bmc(cx, enabled).await,
            LockdownScope::All => {
                let host = set_host(cx, enabled).await?;
                Ok(host.merge(set_bmc(cx, enabled).await?))
            }
        }
    }
}
