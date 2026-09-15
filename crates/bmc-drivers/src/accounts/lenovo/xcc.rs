/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use serde_json::json;

use crate::accounts::standard::{StandardAccounts, apply_policy};

/// Lenovo XCC rejects a zero lockout duration and enforces password rotation
/// through `Oem.Lenovo`, which the default policy disables.
pub(crate) struct XccAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for XccAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        apply_policy(
            cx,
            &json!({
                "AccountLockoutThreshold": 0,
                "AccountLockoutDuration": 60,
                "Oem": {
                    "Lenovo": {
                        "PasswordExpirationPeriodDays": 0,
                        "PasswordChangeOnFirstAccess": false,
                        "MinimumPasswordChangeIntervalHours": 0,
                        "MinimumPasswordReuseCycle": 0,
                        "PasswordExpirationWarningPeriod": 0
                    }
                }
            }),
        )
        .await
    }
}
