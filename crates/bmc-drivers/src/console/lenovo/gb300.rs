/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::ConsoleSpec;

use super::super::support::{AttrExpectation, BiosAttributeConsole, attr};

/// Lenovo GB300 AMI console; this BIOS prefixes enum values with the attribute name.
pub(crate) static GB300_CONSOLE: BiosAttributeConsole = BiosAttributeConsole {
    attrs: ATTRS,
    write_only: &[],
    spec: || {
        Ok(ConsoleSpec::None {
            reason: "GB300 console transport is not identified".to_string(),
        })
    },
};

const ATTRS: &[AttrExpectation] = &[
    attr("TER001", &["Enabled"], &["Disabled"]),
    attr("TER010", &["Enabled"], &["Disabled"]),
    attr("TER06B", &["TER06BCOM0"], &[]),
    attr("TER0021", &["TER0021115200"], &[]),
    attr("TER0020", &["TER0020115200"], &[]),
    attr("TER012", &["TER012VT100Plus"], &[]),
    attr("TER011", &["TER011VTUTF8"], &[]),
    attr("TER05D", &["TER05DNone"], &[]),
];
