/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::{ConsoleFallback, ConsoleSpec, EscapeSeq, PlatformError};

use super::super::support::{
    AttrExpectation, BiosAttributeConsole, SSH_PORT, attr, optional_attr, spec_error,
};

/// Lenovo XClarity Controller console.
pub(crate) static XCC_CONSOLE: BiosAttributeConsole = BiosAttributeConsole {
    attrs: ATTRS,
    write_only: &[],
    spec: xcc_spec,
};

const ATTRS: &[AttrExpectation] = &[
    attr("DevicesandIOPorts_COMPort1", &["Enabled"], &[]),
    attr(
        "DevicesandIOPorts_ConsoleRedirection",
        &["Enabled"],
        &["Auto"],
    ),
    attr(
        "DevicesandIOPorts_SerialPortSharing",
        &["Enabled"],
        &["Disabled"],
    ),
    attr(
        "DevicesandIOPorts_SerialPortAccessMode",
        &["Shared"],
        &["Disabled"],
    ),
    optional_attr(
        "DevicesandIOPorts_SPRedirection",
        &["Enabled"],
        &["Disabled"],
    ),
    optional_attr(
        "DevicesandIOPorts_COMPortActiveAfterBoot",
        &["Enabled"],
        &["Disabled"],
    ),
];

fn xcc_spec() -> Result<ConsoleSpec, PlatformError> {
    // A stale session makes `console 1` fail; the fallback kills and restarts it.
    let fallback = ConsoleFallback::new(
        b"The command line contains extraneous arguments".to_vec(),
        vec![b"console kill".to_vec(), b"console start".to_vec()],
    )
    .map_err(spec_error)?;
    ConsoleSpec::ssh_shell(
        SSH_PORT,
        b"console kill 1\nconsole 1".to_vec(),
        Some(fallback),
        b"\nsystem>".to_vec(),
        EscapeSeq::pair(0x1b, vec![0x28]).map_err(spec_error)?,
    )
    .map_err(spec_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_constructs_the_shell_with_its_fallback() {
        let spec = xcc_spec().expect("Lenovo spec");
        let shell = spec.as_ssh_shell().expect("SSH shell");
        assert_eq!(shell.activate.as_slice(), b"console kill 1\nconsole 1");
        assert_eq!(
            shell
                .fallback
                .as_ref()
                .expect("fallback")
                .commands()
                .collect::<Vec<_>>(),
            vec![b"console kill".as_slice(), b"console start".as_slice()]
        );
    }
}
