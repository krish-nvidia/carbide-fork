/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Account policies shared by several vendor drivers.

use nv_redfish::account::AccountServiceUpdate;

/// The smallest lockout OpenBMC-derived firmware accepts: ten failures, ten minutes.
pub(super) fn openbmc_minimum_lockout_policy() -> AccountServiceUpdate {
    AccountServiceUpdate::builder()
        .with_account_lockout_threshold(10)
        .with_account_lockout_duration(600)
        .build()
}
