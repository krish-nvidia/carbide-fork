/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Console, ConsoleSpec, ConsoleStatus, DriverOutcome, EscapeSeq, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;
use serde_json::{Value, json};

use super::super::support::{
    AttrExpectation, SSH_PORT, attr, attr_status, bios_attributes, optional_attr, spec_error,
};
use crate::dell;
use crate::resources::patch_bios_settings;

/// Dell iDRAC console driver.
///
/// Serial redirection is split between BIOS attributes and iDRAC manager
/// attributes, and the console is reached through the `racadm` SSH shell.
pub(crate) struct IdracConsole;

const ATTRS: &[AttrExpectation] = &[
    attr(
        "SerialComm",
        &[
            "OnConRedir",
            "OnConRedirAuto",
            "OnConRedirCom1",
            "OnConRedirCom2",
        ],
        &["Off"],
    ),
    attr(
        "SerialPortAddress",
        &["Com1", "Serial1Com2Serial2Com1"],
        &[],
    ),
    attr("ExtSerialConnector", &["Serial1"], &[]),
    attr("FailSafeBaud", &["115200"], &[]),
    attr("ConTermType", &["Vt100Vt220"], &[]),
    optional_attr("RedirAfterBoot", &["Enabled"], &["Disabled"]),
    attr("SSH.1.Enable", &["Enabled"], &["Disabled"]),
    attr("SerialRedirection.1.Enable", &["Enabled"], &["Disabled"]),
    attr("IPMISOL.1.Enable", &["Enabled"], &["Disabled"]),
    attr("IPMISOL.1.BaudRate", &["115200"], &[]),
    attr("IPMISOL.1.MinPrivilege", &["Administrator"], &[]),
];

const MANAGER_ATTRS: [&str; 5] = [
    "SSH.1.Enable",
    "SerialRedirection.1.Enable",
    "IPMISOL.1.Enable",
    "IPMISOL.1.BaudRate",
    "IPMISOL.1.MinPrivilege",
];

fn dell_spec() -> Result<ConsoleSpec, PlatformError> {
    ConsoleSpec::ssh_shell(
        SSH_PORT,
        b"connect com2".to_vec(),
        None,
        b"\nracadm>>".to_vec(),
        EscapeSeq::Single(0x1c),
    )
    .map_err(spec_error)
}

#[async_trait]
impl<B: Bmc> Console<B> for IdracConsole {
    async fn setup(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        let attrs = bios_attributes(cx).await?;
        // Newer BIOS generations expose a second serial address encoding.
        let newer = attrs
            .get("SerialPortAddress")
            .and_then(Value::as_str)
            .is_some_and(|value| value.starts_with("Serial1"));
        let mut payload = json!({
            "SerialComm": if newer { "OnConRedirAuto" } else { "OnConRedir" },
            "SerialPortAddress": if newer { "Serial1Com2Serial2Com1" } else { "Com1" },
            "ExtSerialConnector": "Serial1",
            "FailSafeBaud": "115200",
            "ConTermType": "Vt100Vt220"
        });
        if attrs.contains_key("RedirAfterBoot") {
            payload["RedirAfterBoot"] = "Enabled".into();
        }
        let bios_outcome = patch_bios_settings(cx, &json!({"Attributes": payload}))
            .await
            .map(dell::job_outcome)?;
        let manager_outcome = dell::patch_manager_attributes(
            cx,
            json!({
                "SerialRedirection.1.Enable": "Enabled",
                "IPMISOL.1.Enable": "Enabled",
                "IPMISOL.1.BaudRate": "115200",
                "IPMISOL.1.MinPrivilege": "Administrator",
                "SSH.1.Enable": "Enabled",
                "IPMILan.1.Enable": "Enabled"
            }),
        )
        .await?;
        Ok(bios_outcome.merge(manager_outcome))
    }

    async fn status(&self, cx: &OpCx<'_, B>) -> Result<ConsoleStatus, PlatformError> {
        let mut attrs = bios_attributes(cx).await?;
        let dell = cx
            .manager()?
            .oem_dell_attributes()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        for key in MANAGER_ATTRS {
            if let Some(value) = dell
                .attribute(key)
                .and_then(|value| value.str_value().map(str::to_owned))
            {
                attrs.insert(key.to_string(), Value::String(value));
            }
        }
        Ok(attr_status(&attrs, ATTRS))
    }

    async fn spec(&self, _cx: &OpCx<'_, B>) -> Result<ConsoleSpec, PlatformError> {
        dell_spec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_constructs_the_racadm_shell() {
        let spec = dell_spec().expect("Dell spec");
        let shell = spec.as_ssh_shell().expect("SSH shell");
        assert_eq!(shell.activate.as_slice(), b"connect com2");
        assert_eq!(shell.escape_filter, EscapeSeq::Single(0x1c));
    }
}
