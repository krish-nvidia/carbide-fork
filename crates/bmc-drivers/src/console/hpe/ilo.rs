/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Console, ConsoleSpec, ConsoleStatus, DriverOutcome, EscapeSeq, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;

use crate::console::support::{
    AttrExpectation, SSH_PORT, attr, attr_status, bios_attributes, setup_bios_attributes,
    spec_error,
};

/// HPE iLO console; the virtual serial port is reached with `vsp`.
pub(crate) struct IloConsole;

const ATTRS: &[AttrExpectation] = &[
    attr("EmbeddedSerialPort", &["Com2Irq3"], &["Disabled"]),
    attr("EmsConsole", &["Virtual"], &["Disabled"]),
    attr("SerialConsoleBaudRate", &["BaudRate115200"], &[]),
    attr("SerialConsoleEmulation", &["Vt100Plus"], &[]),
    attr("SerialConsolePort", &["Virtual"], &["Disabled"]),
    attr("VirtualSerialPort", &["Com1Irq4"], &["Disabled"]),
];

/// Attributes `setup` writes that the BIOS does not report back meaningfully.
const WRITE_ONLY: &[(&str, &str)] = &[("UefiSerialDebugLevel", "ErrorsOnly")];

fn hpe_spec() -> Result<ConsoleSpec, PlatformError> {
    ConsoleSpec::ssh_shell(
        SSH_PORT,
        b"vsp".to_vec(),
        None,
        b"\n</>hpiLO->".to_vec(),
        EscapeSeq::pair(0x1b, vec![0x28]).map_err(spec_error)?,
    )
    .map_err(spec_error)
}

#[async_trait]
impl<B: Bmc> Console<B> for IloConsole {
    async fn setup(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        setup_bios_attributes(cx, ATTRS, WRITE_ONLY).await
    }

    async fn status(&self, cx: &OpCx<'_, B>) -> Result<ConsoleStatus, PlatformError> {
        Ok(attr_status(&bios_attributes(cx).await?, ATTRS))
    }

    async fn spec(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleSpec, PlatformError> {
        hpe_spec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_constructs_the_vsp_shell() {
        assert_eq!(
            hpe_spec()
                .expect("HPE spec")
                .as_ssh_shell()
                .expect("SSH shell")
                .activate
                .as_slice(),
            b"vsp"
        );
    }
}
