/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Console, ConsoleSpec, ConsoleState, ConsoleStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;

use super::super::support::ipmi_sol_spec;

/// Supermicro console driver.
///
/// The BMC exposes its serial-console state read-only, so setup can only
/// confirm an already enabled console.
pub(crate) struct SupermicroBmcConsole;

#[async_trait]
impl<B: Bmc> Console<B> for SupermicroBmcConsole {
    async fn setup(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        if self.status(cx).await?.state == ConsoleState::Enabled {
            Ok(DriverOutcome::complete())
        } else {
            Err(PlatformError::Unsupported)
        }
    }

    async fn status(&self, cx: &OpCx<'_, B>) -> Result<ConsoleStatus, PlatformError> {
        let system = cx.system()?;
        let raw = system.raw();
        let serial = raw
            .serial_console
            .as_ref()
            .ok_or(PlatformError::Unsupported)?;
        let ssh_enabled = serial
            .ssh
            .as_ref()
            .is_some_and(|ssh| ssh.service_enabled == Some(true));
        let enabled = ssh_enabled && serial.max_concurrent_sessions != Some(0);
        Ok(ConsoleStatus {
            state: if enabled {
                ConsoleState::Enabled
            } else {
                ConsoleState::Disabled
            },
            message: format!(
                "ssh_service_enabled={ssh_enabled}, max_concurrent_sessions={:?}",
                serial.max_concurrent_sessions
            ),
        })
    }

    async fn spec(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleSpec, PlatformError> {
        ipmi_sol_spec()
    }
}
