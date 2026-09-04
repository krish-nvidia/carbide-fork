/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::accounts::standard::StandardAccounts;

/// NVIDIA BlueField: standard account operations; the DPU BMC exposes its
/// lockout policy read-only.
pub(crate) static BLUEFIELD_ACCOUNTS: StandardAccounts = StandardAccounts {
    default_policy: None,
};
