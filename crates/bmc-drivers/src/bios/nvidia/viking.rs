/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::bios::standard::{PasswordMethod, ResetMethod, StandardBios};

/// NVIDIA DGX Viking: AMI firmware naming the password `AdminPassword`; BIOS
/// defaults are restored by clearing the host BIOS NVRAM through the
/// UpdateService OEM action.
pub(crate) static VIKING_BIOS: StandardBios = StandardBios {
    password: PasswordMethod::Action {
        name: "AdminPassword",
    },
    reset: ResetMethod::ClearNvram,
};
