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
use nv_redfish::host_interface::HostInterface;
use nv_redfish::manager::Manager;
use nv_redfish::oem::supermicro::kcs_interface::Privilege;
use serde_json::json;

use crate::lockdown::support::{signal, state_from_signals, status};

/// Supermicro lockdown driver.
///
/// Host lockdown drops KCS to callback privilege and disables the host
/// interfaces; BMC lockdown is the OEM `SysLockdown` switch, which must be
/// cleared before any other write and set after them.
pub(crate) struct SmcLockdown {
    /// Whether the manager's host interfaces are part of host lockdown.
    pub(crate) host_interface_is_lock_control: bool,
}

pub(crate) static SMC_LOCKDOWN: SmcLockdown = SmcLockdown {
    host_interface_is_lock_control: true,
};

/// ARS-121L-DNR loses BMC reachability when its host interface is disabled,
/// so that model keeps the interface up while locked.
pub(crate) static ARS121L_LOCKDOWN: SmcLockdown = SmcLockdown {
    host_interface_is_lock_control: false,
};

fn bmc_scope(scope: LockdownScope) -> bool {
    matches!(
        scope,
        LockdownScope::Bmc | LockdownScope::BmcSystemLockdown | LockdownScope::All
    )
}

async fn set_sys_lockdown<B: Bmc>(
    cx: &OpCx<'_, B>,
    manager: &Manager<B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let lockdown = manager
        .oem_supermicro()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .sys_lockdown()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let raw = lockdown.raw();
    cx.patch(raw.as_ref(), &json!({"SysLockdownEnabled": enabled}))
        .await
}

async fn set_kcs_privilege<B: Bmc>(
    cx: &OpCx<'_, B>,
    manager: &Manager<B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let kcs = manager
        .oem_supermicro()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .kcs_interface()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let raw = kcs.raw();
    cx.patch(
        raw.as_ref(),
        &json!({"Privilege": if enabled { "Callback" } else { "Administrator" }}),
    )
    .await
}

async fn host_interfaces<B: Bmc>(
    cx: &OpCx<'_, B>,
    manager: &Manager<B>,
) -> Result<Vec<HostInterface<B>>, PlatformError> {
    manager
        .host_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

#[async_trait]
impl<B: Bmc> Lockdown<B> for SmcLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let manager = cx.manager()?;
        let smc = manager
            .oem_supermicro()
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        let lockdown = smc
            .sys_lockdown()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .and_then(|value| value.sys_lockdown_enabled());
        let privilege = smc
            .kcs_interface()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .and_then(|value| value.privilege());
        let host_interface = host_interfaces(cx, manager)
            .await?
            .first()
            .and_then(|value| value.interface_enabled());

        let mut signals = vec![signal(
            privilege,
            Privilege::Callback,
            Privilege::Administrator,
        )];
        if self.host_interface_is_lock_control {
            signals.push(signal(host_interface, false, true));
        }
        let host = state_from_signals(&signals);
        let bmc = state_from_signals(&[signal(lockdown, true, false)]);
        Ok(status(
            host,
            bmc,
            format!(
                "sys_lockdown={lockdown:?}, kcs={privilege:?}, host_interface={host_interface:?}"
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
        let manager = cx.manager()?;
        let mut outcome = DriverOutcome::complete();
        if !enabled && bmc_scope(scope) {
            outcome = outcome.merge(set_sys_lockdown(cx, manager, false).await?);
        }
        if matches!(scope, LockdownScope::Host | LockdownScope::All) {
            outcome = outcome.merge(set_kcs_privilege(cx, manager, enabled).await?);
            if self.host_interface_is_lock_control || !enabled {
                for interface in host_interfaces(cx, manager).await? {
                    outcome = outcome.merge(
                        cx.patch_id(
                            interface.odata_id(),
                            None,
                            &json!({"InterfaceEnabled": !enabled}),
                        )
                        .await
                        .map(DriverOutcome::from)?,
                    );
                }
            }
        }
        if enabled && bmc_scope(scope) {
            outcome = outcome.merge(set_sys_lockdown(cx, manager, true).await?);
        }
        Ok(outcome)
    }
}
