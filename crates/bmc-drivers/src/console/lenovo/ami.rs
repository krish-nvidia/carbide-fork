/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Console, ConsoleSpec, ConsoleState, ConsoleStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;

use crate::console::support::SSH_PORT;

/// Lenovo AMI: SSH login lands on the serial console directly, so there is
/// nothing to set up.
pub(crate) struct LenovoAmiConsole;

#[async_trait]
impl<B: Bmc> Console<B> for LenovoAmiConsole {
    async fn setup(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }

    async fn status(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleStatus, PlatformError> {
        Ok(ConsoleStatus {
            state: ConsoleState::Enabled,
            message: "SSH login opens the serial console directly".to_string(),
        })
    }

    async fn spec(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleSpec, PlatformError> {
        Ok(ConsoleSpec::SshDirect { port: SSH_PORT })
    }
}
