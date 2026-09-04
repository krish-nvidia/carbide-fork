/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use super::super::support::{AttrExpectation, BiosAttributeConsole, attr, ipmi_sol_spec};

/// NVIDIA Viking console; the console is an IPMI SOL session.
pub(crate) static VIKING_CONSOLE: BiosAttributeConsole = BiosAttributeConsole {
    attrs: ATTRS,
    write_only: &[
        ("AcpiSpcrTerminalType", "VT-UTF8"),
        ("TerminalType0", "ANSI"),
    ],
    spec: ipmi_sol_spec,
};

const ATTRS: &[AttrExpectation] = &[
    attr("AcpiSpcrConsoleRedirectionEnable", &["true"], &["false"]),
    attr("ConsoleRedirectionEnable0", &["true"], &["false"]),
    attr("AcpiSpcrPort", &["COM0"], &[]),
    attr("AcpiSpcrFlowControl", &["None"], &[]),
    attr("AcpiSpcrBaudRate", &["115200"], &[]),
    attr("BaudRate0", &["115200"], &[]),
];
