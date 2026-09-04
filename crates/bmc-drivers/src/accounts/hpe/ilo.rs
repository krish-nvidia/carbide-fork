/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::{Value, json};

use crate::accounts::standard::StandardAccounts;

/// HPE iLO: standard account operations; lockout policy lives under `Oem.Hpe`.
pub(crate) static ILO_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: Some(policy),
};

fn policy() -> Value {
    json!({
        "Oem": {
            "Hpe": {
                "AuthFailureDelayTimeSeconds": 2,
                "AuthFailureLoggingThreshold": 0,
                "AuthFailuresBeforeDelay": 0,
                "EnforcePasswordComplexity": false
            }
        }
    })
}
