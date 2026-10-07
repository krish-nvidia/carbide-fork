/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Console, ConsoleSpec, ConsoleState, ConsoleStatus, DriverOutcome, EscapeSeq, OpCx,
    PlatformError,
};
use nv_redfish::core::Bmc;

use crate::console::support::{
    AttrExpectation, RedfishConsoleExt as _, SSH_PORT, spec_error, write_only,
};

/// HPE iLO console; the virtual serial port is reached with `vsp`. Status
/// reports the console enabled without checking the BIOS settings.
pub(crate) struct IloConsole;

const ATTRS: &[AttrExpectation] = &[
    write_only("EmbeddedSerialPort", &["Com2Irq3"]),
    write_only("EmsConsole", &["Virtual"]),
    write_only("SerialConsoleBaudRate", &["BaudRate115200"]),
    write_only("SerialConsoleEmulation", &["Vt100Plus"]),
    write_only("SerialConsolePort", &["Virtual"]),
    write_only("UefiSerialDebugLevel", &["ErrorsOnly"]),
    write_only("VirtualSerialPort", &["Com1Irq4"]),
];

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
        cx.setup_console_bios_attributes(ATTRS).await
    }

    async fn status(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleStatus, PlatformError> {
        Ok(ConsoleStatus {
            state: ConsoleState::Enabled,
            message: String::new(),
        })
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
