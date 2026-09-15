/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use serde_json::json;

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
            &json!({
                "AccountLockoutThreshold": 4,
                "AccountLockoutDuration": 20,
                "AccountLockoutCounterResetAfter": 20,
                "AccountLockoutCounterResetEnabled": true,
                "AuthFailureLoggingThreshold": 2
            }),
        )
        .await
    }
}
