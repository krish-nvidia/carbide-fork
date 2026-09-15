/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Account policies shared by several vendor drivers.

use serde_json::{Value, json};

/// The smallest lockout OpenBMC-derived firmware accepts: ten failures, ten minutes.
pub(super) fn openbmc_minimum_lockout_policy() -> Value {
    json!({
        "AccountLockoutThreshold": 10,
        "AccountLockoutDuration": 600
    })
}
