/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::accounts::standard::{StandardAccounts, openbmc_minimum_lockout_policy};

/// NVIDIA GB NVSwitch trays: standard account operations with the OpenBMC lockout policy.
pub(crate) static SWITCH_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: Some(openbmc_minimum_lockout_policy),
};
