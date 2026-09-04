/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::{Value, json};

use crate::accounts::standard::StandardAccounts;

/// NVIDIA OpenBMC trays: standard account operations. Newer tray firmware
/// rejects a zero lockout threshold, so the policy keeps a short lockout.
pub(crate) static OPENBMC_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: Some(policy),
};

fn policy() -> Value {
    json!({
        "AccountLockoutThreshold": 4,
        "AccountLockoutDuration": 600
    })
}
