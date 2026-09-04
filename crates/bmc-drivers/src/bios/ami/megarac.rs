/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::bios::standard::{PasswordMethod, ResetMethod, StandardBios};

/// AMI MegaRAC (including Lenovo AMI-based systems) names the UEFI
/// administrator password `SETUP001`.
pub(crate) static MEGARAC_BIOS: StandardBios = StandardBios {
    password: PasswordMethod::Action { name: "SETUP001" },
    reset: ResetMethod::Action,
};
