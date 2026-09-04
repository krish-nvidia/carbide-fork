/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::bios::standard::{PasswordMethod, ResetMethod, StandardBios};

/// NVIDIA BlueField: the DPU BIOS exposes no `ResetBios` or `ChangePassword`
/// actions; both are write-only attributes on the pending settings.
pub(crate) static BLUEFIELD_BIOS: StandardBios = StandardBios {
    password: PasswordMethod::PendingAttributes,
    reset: ResetMethod::PendingAttribute("ResetEfiVars"),
};
