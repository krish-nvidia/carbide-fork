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
use nv_redfish::ethernet_interface::{EthernetInterface, EthernetInterfaceUpdate};
use nv_redfish::manager::Manager;
use nv_redfish::oem::lenovo::computer_system::{
    FpMode, LenovoSystemPropertiesUpdate, PortSwitchingTo, UsbManagementPortAssignmentUpdate,
};
use nv_redfish::oem::lenovo::manager::KcsState;
use nv_redfish::oem::lenovo::security_service::FwRollbackState;

use crate::lockdown::{signal, state_from_signals, status};

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
        .find(|interface| interface.raw().id == HOST_INTERFACE_ID))
}

async fn set_host<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    let manager = cx.manager()?;
    let lenovo = manager
        .oem_lenovo()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let kcs = lenovo
        .set_kcs_enabled(!enabled)
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .map(DriverOutcome::from)
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "XCC manager does not report KCSEnabled".to_string(),
        })?;

    let rollback = lenovo
        .security()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .set_fw_rollback(if enabled {
            FwRollbackState::Disabled
        } else {
            FwRollbackState::Enabled
        })
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))?;

    let to_host = host_interface(cx, manager)
        .await?
        .ok_or(PlatformError::Unsupported)?;
    let body = EthernetInterfaceUpdate::builder()
        .with_interface_enabled(!enabled)
        .build();
    to_host
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
        .map(|to_host| kcs.merge(rollback).merge(to_host))
}

/// Newer XCC renamed `FrontPanelUSB` to `USBManagementPortAssignment`;
/// `FrontPanelUSB` is written whenever the system reports it.
async fn set_bmc<B: Bmc>(cx: &OpCx<'_, B>, enabled: bool) -> Result<DriverOutcome, PlatformError> {
    let lenovo = cx
        .system()?
        .oem_lenovo()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let assignment = UsbManagementPortAssignmentUpdate::builder()
        .with_fp_mode(if enabled {
            FpMode::Server
        } else {
            FpMode::Shared
        })
        .with_port_switching_to(PortSwitchingTo::Server)
        .build();
    let raw = lenovo.raw();
    let update = if raw.front_panel_usb.is_some() {
        LenovoSystemPropertiesUpdate::builder().with_front_panel_usb(assignment)
    } else if raw.usb_management_port_assignment.is_some() {
        LenovoSystemPropertiesUpdate::builder().with_usb_management_port_assignment(assignment)
    } else {
        return Err(PlatformError::InvalidResponse {
            message: "XCC system reports neither FrontPanelUSB nor USBManagementPortAssignment"
                .to_string(),
        });
    };
    lenovo
        .update(&update.build())
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
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
