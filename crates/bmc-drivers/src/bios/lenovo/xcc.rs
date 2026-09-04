/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::bios::standard::{PasswordMethod, ResetMethod, StandardBios};

/// Lenovo XCC names the UEFI administrator password `UefiAdminPassword`.
pub(crate) static XCC_BIOS: StandardBios = StandardBios {
    password: PasswordMethod::Action {
        name: "UefiAdminPassword",
    },
    reset: ResetMethod::Action,
};
