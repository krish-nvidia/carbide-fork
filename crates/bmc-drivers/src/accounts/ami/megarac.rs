/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::accounts::standard::{StandardAccounts, no_lockout_policy};

/// AMI MegaRAC: standard account operations with the default no-lockout policy.
pub(crate) static MEGARAC_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: Some(no_lockout_policy),
};
