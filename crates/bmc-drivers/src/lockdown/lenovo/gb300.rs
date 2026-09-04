/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::lockdown::support::{BiosAttributeLockdown, LockedAttribute};

/// Lenovo GB300 AMI: like MegaRAC, except this BIOS prefixes enum values
/// with the attribute name and exposes no KCS access control.
pub(crate) static GB300_LOCKDOWN: BiosAttributeLockdown = BiosAttributeLockdown {
    host: &[LockedAttribute {
        key: "USB000",
        locked: "USB000Disabled",
        unlocked: "USB000Enabled",
    }],
};
