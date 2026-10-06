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

/// Supermicro ARS-121L-DNR lockdown driver.
///
/// Like the other SMC boards, host lockdown drops KCS to callback privilege
/// and BMC lockdown is the OEM `SysLockdown` switch, cleared before any other
/// write and set after them. This model loses BMC reachability when its host
/// interface is disabled, so the interface is left up while locked and only
/// re-enabled on unlock.
pub(crate) struct Ars121lLockdown;

#[async_trait]
impl<B: Bmc> Lockdown<B> for Ars121lLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let lockdown = sys_lockdown(cx).await?;
        let privilege = kcs_privilege(cx).await?;
        let host_interface = host_interface_enabled(cx).await?;

        // The host interface stays up while locked, so it only gates unlocking.
        let host =
            state_from_signals(&[kcs_signal(privilege), (true, host_interface == Some(true))]);
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
            let privilege = if enabled {
                Privilege::Callback
            } else {
                Privilege::Administrator
            };
            outcome = outcome.merge(set_kcs_privilege(cx, privilege).await?);
            if !enabled {
                outcome = outcome.merge(set_host_interfaces(cx, true).await?);
            }
        }
        if enabled && sys_lockdown {
            outcome = outcome.merge(set_sys_lockdown(cx, true).await?);
        }
        Ok(outcome)
    }
}
