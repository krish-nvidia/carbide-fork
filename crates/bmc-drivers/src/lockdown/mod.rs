/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Host and BMC lockdown capability drivers.

use bmc_platform::{DriverOutcome, LockdownState, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::host_interface::HostInterface;
use serde_json::json;

mod ami;
mod dell;
mod hpe;
mod lenovo;
mod nvidia;
mod supermicro;
mod support;

pub(crate) use ami::MEGARAC_LOCKDOWN;
pub(crate) use dell::IdracLockdown;
pub(crate) use hpe::IloLockdown;
pub(crate) use lenovo::{GB300_LOCKDOWN, LenovoAmiLockdown, XccLockdown};
pub(crate) use nvidia::{OpenBmcLockdown, VikingLockdown};
pub(crate) use supermicro::{ARS121L_LOCKDOWN, SMC_LOCKDOWN};

async fn host_interfaces<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Vec<HostInterface<B>>, PlatformError> {
    cx.manager()?
        .host_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

/// BMC lockdown state derived from the manager's first host interface.
async fn host_interface_state<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<(LockdownState, Option<bool>), PlatformError> {
    let enabled = host_interfaces(cx)
        .await?
        .first()
        .and_then(|interface| interface.interface_enabled());
    Ok((
        support::state_from_signals(&[support::signal(enabled, false, true)]),
        enabled,
    ))
}

async fn set_first_host_interface<B: Bmc>(
    cx: &OpCx<'_, B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let interface = host_interfaces(cx)
        .await?
        .into_iter()
        .next()
        .ok_or(PlatformError::NoContent)?;
    let raw = interface.raw();
    cx.patch(raw.as_ref(), &json!({"InterfaceEnabled": enabled}))
        .await
}
