/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::lockdown::support::{BiosAttributeLockdown, LockedAttribute};

/// Generic AMI MegaRAC: host lockdown is KCS access and USB support in BIOS;
/// BMC lockdown is the manager's host interface.
pub(crate) static MEGARAC_LOCKDOWN: BiosAttributeLockdown = BiosAttributeLockdown {
    host: &[
        LockedAttribute {
            key: "KCSACP",
            locked: "Deny All",
            unlocked: "Allow All",
        },
        LockedAttribute {
            key: "USB000",
            locked: "Disabled",
            unlocked: "Enabled",
        },
    ],
};
