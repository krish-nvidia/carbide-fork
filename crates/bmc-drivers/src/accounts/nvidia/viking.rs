/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::{Value, json};

use crate::accounts::standard::StandardAccounts;

/// NVIDIA DGX Viking: the firmware rejects a fully disabled lockout, so the
/// policy keeps a short, self-resetting one.
pub(crate) static VIKING_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: Some(policy),
};

fn policy() -> Value {
    json!({
        "AccountLockoutThreshold": 4,
        "AccountLockoutDuration": 20,
        "AccountLockoutCounterResetAfter": 20,
        "AccountLockoutCounterResetEnabled": true,
        "AuthFailureLoggingThreshold": 2
    })
}
