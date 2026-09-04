/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::{ConsoleSpec, EscapeSeq, PlatformError};

use super::super::support::{AttrExpectation, BiosAttributeConsole, SSH_PORT, attr, spec_error};

/// HPE iLO console; the virtual serial port is reached with `vsp`.
pub(crate) static ILO_CONSOLE: BiosAttributeConsole = BiosAttributeConsole {
    attrs: ATTRS,
    write_only: &[("UefiSerialDebugLevel", "ErrorsOnly")],
    spec: hpe_spec,
};

const ATTRS: &[AttrExpectation] = &[
    attr("EmbeddedSerialPort", &["Com2Irq3"], &["Disabled"]),
    attr("EmsConsole", &["Virtual"], &["Disabled"]),
    attr("SerialConsoleBaudRate", &["BaudRate115200"], &[]),
    attr("SerialConsoleEmulation", &["Vt100Plus"], &[]),
    attr("SerialConsolePort", &["Virtual"], &["Disabled"]),
    attr("VirtualSerialPort", &["Com1Irq4"], &["Disabled"]),
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
