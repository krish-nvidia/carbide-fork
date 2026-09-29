/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::AccountServiceUpdate;
use nv_redfish::core::Bmc;

use crate::accounts::standard::{StandardAccounts, apply_policy};

/// NVIDIA DGX Viking: the firmware rejects a fully disabled lockout, so the
/// policy keeps a short, self-resetting one.
pub(crate) struct VikingAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for VikingAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        apply_policy(
            cx,
            AccountServiceUpdate::builder()
                .with_account_lockout_threshold(4)
                .with_account_lockout_duration(20)
                .with_account_lockout_counter_reset_after(20)
                .with_account_lockout_counter_reset_enabled(true)
                .with_auth_failure_logging_threshold(2)
                .build(),
        )
        .await
    }
}
