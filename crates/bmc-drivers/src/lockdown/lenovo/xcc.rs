/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Lockdown, LockdownDesiredState, LockdownScope, LockdownStatus, OpCx,
    PlatformError,
};
use nv_redfish::Resource;
use nv_redfish::core::Bmc;
use nv_redfish::ethernet_interface::EthernetInterface;
use nv_redfish::manager::Manager;
use nv_redfish::oem::lenovo::computer_system::{FpMode, PortSwitchingTo};
use nv_redfish::oem::lenovo::manager::{KcsState, LenovoManagerSchema};
use nv_redfish::oem::lenovo::security_service::FwRollbackState;
use serde_json::{Value, json};

use crate::lockdown::support::{signal, state_from_signals, status};

/// The manager Ethernet interface XCC exposes to the host OS.
const HOST_INTERFACE_ID: &str = "ToHost";

/// Lenovo XClarity Controller lockdown driver.
///
/// Host lockdown disables KCS, firmware rollback, and the host-facing
/// Ethernet interface; BMC lockdown dedicates the front-panel USB port to
/// the server.
pub(crate) struct XccLockdown;

async fn host_interface<B: Bmc>(
    cx: &OpCx<'_, B>,
    manager: &Manager<B>,
) -> Result<Option<EthernetInterface<B>>, PlatformError> {
    Ok(manager
        .ethernet_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .find(|interface| interface.id().into_inner() == HOST_INTERFACE_ID))
}

async fn set_host<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    let manager = cx.manager()?;
    let lenovo = manager
        .oem_lenovo()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    // Older XCC firmware models KCSEnabled as a boolean, newer as an enum string.
    let kcs_enabled = match lenovo.raw().as_ref() {
        LenovoManagerSchema::V0_1(_) => Value::Bool(!enabled),
        LenovoManagerSchema::V1_0(_) => Value::from(if enabled { "Disabled" } else { "Enabled" }),
    };
    let raw = manager.raw();
    let kcs = cx
        .patch(
            raw.as_ref(),
            &json!({"Oem": {"Lenovo": {"KCSEnabled": kcs_enabled}}}),
        )
        .await?;

    let security = lenovo
        .base()
        .security
        .as_ref()
        .ok_or(PlatformError::Unsupported)?
        .id()
        .clone();
    let rollback = cx
        .patch_id(
            &security,
            None,
            &json!({"Configurator": {"FWRollback": if enabled { "Disabled" } else { "Enabled" }}}),
        )
        .await
        .map(DriverOutcome::from)?;

    let to_host = host_interface(cx, manager)
        .await?
        .ok_or(PlatformError::Unsupported)?;
    cx.patch_id(
        to_host.odata_id(),
        None,
        &json!({"InterfaceEnabled": !enabled}),
    )
    .await
    .map(DriverOutcome::from)
    .map(|to_host| kcs.merge(rollback).merge(to_host))
}

async fn set_bmc<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    let system = cx.system()?;
    // Newer XCC renamed the front-panel object; write whichever this system exposes.
    let front_panel_key = system
        .oem_lenovo()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .raw()
        .usb_management_port_assignment
        .as_ref()
        .and_then(Option::as_ref)
        .map_or("FrontPanelUSB", |_| "USBManagementPortAssignment");
    let raw = system.raw();
    cx.patch(
        raw.as_ref(),
        &json!({
            "Oem": {"Lenovo": {front_panel_key: {
                "FPMode": if enabled { "Server" } else { "Shared" },
                "PortSwitchingTo": "Server"
            }}}
        }),
    )
    .await
}

#[async_trait]
impl<B: Bmc> Lockdown<B> for XccLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let manager = cx.manager()?;
        let lenovo = manager
            .oem_lenovo()
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        let kcs = lenovo.kcs_enabled();
        let rollback = lenovo
            .security()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .and_then(|security| security.fw_rollback());
        let to_host = host_interface(cx, manager)
            .await?
            .and_then(|interface| interface.interface_enabled());

        let lenovo_system = cx
            .system()?
            .oem_lenovo()
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        let fp_mode = lenovo_system.front_panel_mode();
        let switching = lenovo_system.port_switching_to();

        let host = state_from_signals(&[
            signal(kcs, KcsState::Disabled, KcsState::Enabled),
            signal(
                rollback,
                FwRollbackState::Disabled,
                FwRollbackState::Enabled,
            ),
            signal(to_host, false, true),
        ]);
        let bmc = state_from_signals(&[(
            fp_mode == Some(FpMode::Server),
            fp_mode == Some(FpMode::Shared) && switching == Some(PortSwitchingTo::Server),
        )]);
        Ok(status(
            host,
            bmc,
            format!(
                "kcs={kcs:?}, firmware_rollback={rollback:?}, to_host={to_host:?}, front_panel={fp_mode:?}/{switching:?}"
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
            LockdownScope::Bmc | LockdownScope::BmcSystemLockdown => set_bmc(cx, enabled).await,
            LockdownScope::All => {
                let host = set_host(cx, enabled).await?;
                Ok(host.merge(set_bmc(cx, enabled).await?))
            }
        }
    }
}
