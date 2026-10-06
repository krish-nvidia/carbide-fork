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
use nv_redfish::oem::supermicro::kcs_interface::Privilege;

use super::support::{
    host_interface_enabled, kcs_privilege, kcs_signal, set_host_interfaces, set_kcs_privilege,
    set_sys_lockdown, sys_lockdown, sys_lockdown_scope,
};
use crate::lockdown::support::{signal, state_from_signals, status};

/// Supermicro lockdown driver.
///
/// Host lockdown disables the host interfaces and then drops KCS to callback
/// privilege, and unlocking reverses that order; BMC lockdown is the OEM
/// `SysLockdown` switch, which must be cleared before any other write and set
/// after them.
pub(crate) struct SmcLockdown;

#[async_trait]
impl<B: Bmc> Lockdown<B> for SmcLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let lockdown = sys_lockdown(cx).await?;
        let privilege = kcs_privilege(cx).await?;
        let host_interface = host_interface_enabled(cx).await?;

        let host =
            state_from_signals(&[kcs_signal(privilege), signal(host_interface, false, true)]);
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
        let sys_lockdown = sys_lockdown_scope(scope)?;
        let enabled = desired == LockdownDesiredState::Enabled;
        let mut outcome = DriverOutcome::complete();
        if !enabled && sys_lockdown {
            outcome = outcome.merge(set_sys_lockdown(cx, false).await?);
        }
        if matches!(scope, LockdownScope::Host | LockdownScope::All) {
            if enabled {
                outcome = outcome.merge(set_host_interfaces(cx, false).await?);
                outcome = outcome.merge(set_kcs_privilege(cx, Privilege::Callback).await?);
            } else {
                outcome = outcome.merge(set_kcs_privilege(cx, Privilege::Administrator).await?);
                outcome = outcome.merge(set_host_interfaces(cx, true).await?);
            }
        }
        if enabled && sys_lockdown {
            outcome = outcome.merge(set_sys_lockdown(cx, true).await?);
        }
        Ok(outcome)
    }
}
