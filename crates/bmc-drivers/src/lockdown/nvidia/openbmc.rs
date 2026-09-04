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

use crate::lockdown::host_interfaces;
use crate::lockdown::support::{signal, state_from_signals, status};

/// NVIDIA OpenBMC lockdown driver; every host interface is the single control.
pub(crate) struct OpenBmcLockdown;

#[async_trait]
impl<B: Bmc> Lockdown<B> for OpenBmcLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let interfaces = host_interfaces(cx).await?;
        let enabled: Vec<_> = interfaces
            .iter()
            .map(|interface| interface.interface_enabled())
            .collect();
        let signals: Vec<_> = enabled
            .iter()
            .map(|enabled| signal(*enabled, false, true))
            .collect();
        let state = state_from_signals(&signals);
        Ok(status(state, state, format!("host_interfaces={enabled:?}")))
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        scope: LockdownScope,
        desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        if scope == LockdownScope::BmcSystemLockdown {
            return Err(PlatformError::Unsupported);
        }
        let interfaces = host_interfaces(cx).await?;
        if interfaces.is_empty() {
            return Err(PlatformError::NoContent);
        }
        let interface_enabled = desired == LockdownDesiredState::Disabled;
        let mut outcome = DriverOutcome::complete();
        for interface in interfaces {
            let raw = interface.raw();
            outcome = outcome.merge(
                cx.patch(
                    raw.as_ref(),
                    &json!({"InterfaceEnabled": interface_enabled}),
                )
                .await?,
            );
        }
        Ok(outcome)
    }
}
