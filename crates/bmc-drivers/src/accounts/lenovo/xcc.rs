/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::{Value, json};

use crate::accounts::standard::StandardAccounts;

/// Lenovo XCC: standard account operations. XCC rejects a zero lockout
/// duration and enforces password rotation through `Oem.Lenovo`, which the
/// default policy disables.
pub(crate) static XCC_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: Some(policy),
};

fn policy() -> Value {
    json!({
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
    })
}
