/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::bios::standard::{PasswordMethod, ResetMethod, StandardBios};

/// NVIDIA OpenBMC platforms (GB200/GB300, Vera Rubin, GH) name the UEFI
/// administrator password `AdminPassword`.
pub(crate) static OPENBMC_BIOS: StandardBios = StandardBios {
    password: PasswordMethod::Action {
        name: "AdminPassword",
    },
    reset: ResetMethod::Action,
};
