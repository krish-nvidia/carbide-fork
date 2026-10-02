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

/// Platforms with nothing to lock down (power shelves, GB NVSwitch trays):
/// every request completes without a write and there is no status to report.
pub(crate) struct NoopLockdown;

#[async_trait]
impl<B: Bmc> Lockdown<B> for NoopLockdown {
    async fn status(&self, _cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn set(
        &self,
        _cx: &OpCx<'_, B>,
        _scope: LockdownScope,
        _desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }
}
