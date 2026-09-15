/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Console, ConsoleSpec, ConsoleStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::console::support::{
    AttrExpectation, attr, attr_status, bios_attributes, ipmi_sol_spec, setup_bios_attributes,
};

/// NVIDIA Viking console; the console is an IPMI SOL session.
pub(crate) struct VikingConsole;

const ATTRS: &[AttrExpectation] = &[
    attr("AcpiSpcrConsoleRedirectionEnable", &["true"], &["false"]),
    attr("ConsoleRedirectionEnable0", &["true"], &["false"]),
    attr("AcpiSpcrPort", &["COM0"], &[]),
    attr("AcpiSpcrFlowControl", &["None"], &[]),
    attr("AcpiSpcrBaudRate", &["115200"], &[]),
    attr("BaudRate0", &["115200"], &[]),
];

/// Attributes `setup` writes that the BIOS does not report back meaningfully.
const WRITE_ONLY: &[(&str, &str)] = &[
    ("AcpiSpcrTerminalType", "VT-UTF8"),
    ("TerminalType0", "ANSI"),
];

#[async_trait]
impl<B: Bmc> Console<B> for VikingConsole {
    async fn setup(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        setup_bios_attributes(cx, ATTRS, WRITE_ONLY).await
    }

    async fn status(&self, cx: &OpCx<'_, B>) -> Result<ConsoleStatus, PlatformError> {
        Ok(attr_status(&bios_attributes(cx).await?, ATTRS))
    }

    async fn spec(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleSpec, PlatformError> {
        ipmi_sol_spec()
    }
}
