/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::ConsoleSpec;

use super::super::support::{AttrExpectation, BiosAttributeConsole, attr};

/// AMI MegaRAC BIOS console attributes.
pub(crate) static MEGARAC_CONSOLE: BiosAttributeConsole = BiosAttributeConsole {
    attrs: ATTRS,
    write_only: &[],
    spec: || {
        Ok(ConsoleSpec::None {
            reason: "AMI console transport is not identified".to_string(),
        })
    },
};

const ATTRS: &[AttrExpectation] = &[
    attr("TER001", &["Enabled"], &["Disabled"]),
    attr("TER010", &["Enabled"], &["Disabled"]),
    attr("TER06B", &["COM1"], &[]),
    attr("TER0021", &["115200"], &[]),
    attr("TER0020", &["115200"], &[]),
    attr("TER012", &["VT100Plus"], &[]),
    attr("TER011", &["VT-UTF8"], &[]),
    attr("TER05D", &["None"], &[]),
];
