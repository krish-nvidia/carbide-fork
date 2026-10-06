/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::collections::BTreeMap;

use async_trait::async_trait;
use bmc_platform::{
    Bios, BiosSettings, BiosStatus, BootInterfaceSelector, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::{Value, json};

use crate::bios::attributes::AttributeValue::{self, Bool, String as Text};
use crate::bios::standard::StandardBios;
use crate::bios::support::{compare, current_settings, with_profile};
use crate::resources::{bios_attributes, bios_update, selected_bios, stage_bios_attributes};

/// Supermicro hosts: the BIOS appends a registry suffix to every attribute
/// name, as in `IPv4HTTPSupport_009F`, and a TPM clear is a pending operation
/// written to the current BIOS resource.
pub(crate) struct SmcBios;

/// Expected values by attribute-name prefix.
const ATTRIBUTES: &[(&str, AttributeValue)] = &[
    ("QuietBoot", Bool(false)),
    ("Re_tryBoot", Text("EFI Boot")),
    ("CSMSupport", Text("Disabled")),
    ("SecureBootEnable", Bool(false)),
    ("TXTSupport", Text("Enabled")),
    ("DeviceSelect", Text("TPM 2.0")),
    ("IntelVTforDirectedI_O_VT_d", Text("Enable")),
    ("IntelVirtualizationTechnology", Text("Enable")),
    ("SR-IOVSupport", Text("Enabled")),
    ("SR_IOVSupport", Text("Enabled")),
    ("IPv4HTTPSupport", Text("Enabled")),
    ("IPv4PXESupport", Text("Disabled")),
    ("IPv6HTTPSupport", Text("Disabled")),
    ("IPv6PXESupport", Text("Disabled")),
];

/// Boards spell this attribute's values either `Enabled`/`Disabled` or
/// `Enable`/`Disable`; it is enabled in the spelling the board uses.
const SECURITY_DEVICE_SUPPORT: &str = "SecurityDeviceSupport";

/// The expected value of every reported attribute the table names.
fn expected_settings(current: &BiosSettings) -> Result<BiosSettings, PlatformError> {
    let mut expected = BiosSettings::default();
    for (name, reported) in &current.attributes {
        let value = if name.starts_with(SECURITY_DEVICE_SUPPORT) {
            security_device_enabled(name, reported)?
        } else if let Some((_, value)) = ATTRIBUTES
            .iter()
            .find(|(prefix, _)| name.starts_with(prefix))
        {
            Value::from(*value)
        } else {
            continue;
        };
        expected.attributes.insert(name.clone(), value);
    }
    Ok(expected)
}

fn security_device_enabled(name: &str, reported: &Value) -> Result<Value, PlatformError> {
    match reported.as_str() {
        Some("Enabled" | "Disabled") => Ok(json!("Enabled")),
        Some("Enable" | "Disable") => Ok(json!("Enable")),
        _ => Err(PlatformError::InvalidResponse {
            message: format!("{name} reports {reported}, which is neither spelling"),
        }),
    }
}

#[async_trait]
impl<B: Bmc> Bios<B> for SmcBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = current_settings(cx).await?;
        stage_bios_attributes(
            cx,
            &with_profile(expected_settings(&current)?, profile).attributes,
        )
        .await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<BiosStatus, PlatformError> {
        let current = current_settings(cx).await?;
        Ok(compare(
            &current,
            &with_profile(expected_settings(&current)?, profile),
        ))
    }

    /// The board names its TPM pending operation `PendingOperation*`, and
    /// takes it on the current BIOS resource rather than the settings resource.
    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        let bios = selected_bios(cx).await?;
        let operation = bios_attributes(&bios.raw())
            .into_keys()
            .find(|key| key.starts_with("PendingOperation"))
            .ok_or(PlatformError::Unsupported)?;
        let attributes = BTreeMap::from([(operation, json!("TPM Clear"))]);
        bios.update(&bios_update(&attributes)?)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Method;
    use carbide_test_support::value_scenarios;

    use super::*;
    use crate::test_support::{Fixture, body, path};

    const SYSTEM: &str = "/redfish/v1/Systems/1";
    const BIOS: &str = "/redfish/v1/Systems/1/Bios";

    #[tokio::test]
    async fn tpm_clear_writes_the_boards_pending_operation_to_the_current_bios() {
        for (attributes, expected) in [
            (
                json!({"PendingOperation_7A3C": "None", "QuietBoot": "Enabled"}),
                Some(json!({"Attributes": {"PendingOperation_7A3C": "TPM Clear"}})),
            ),
            (json!({"QuietBoot": "Enabled"}), None),
        ] {
            let bmc = Fixture::new("Supermicro", "", "1", "1")
                .document(
                    SYSTEM,
                    json!({"@odata.id": SYSTEM, "Id": "1", "Name": "System", "Bios": {"@odata.id": BIOS}}),
                )
                .document(
                    BIOS,
                    json!({"@odata.id": BIOS, "Id": "BIOS", "Name": "BIOS", "Attributes": attributes}),
                )
                .build()
                .await;
            let cx = bmc.cx().await;

            let result = SmcBios.clear_tpm(&cx).await;
            let writes = bmc.writes();
            match expected {
                Some(expected) => {
                    assert_eq!(result, Ok(DriverOutcome::complete()));
                    assert_eq!(writes.len(), 1);
                    assert_eq!(writes[0].method, Method::PATCH);
                    assert_eq!(path(&writes[0]), BIOS);
                    assert_eq!(body(&writes[0]), expected);
                }
                None => {
                    assert_eq!(result, Err(PlatformError::Unsupported));
                    assert!(writes.is_empty());
                }
            }
        }
    }

    #[test]
    fn expected_values_follow_suffixed_names_and_the_boards_spelling() {
        value_scenarios!(run = |(name, reported): (&str, &str)| expected_settings(
            &serde_json::from_value(json!({"attributes": {name: reported, "Unrelated": "x"}}))
                .expect("settings")
        )
        .map(|expected| expected.attributes.into_iter().collect::<Vec<_>>());
            "suffixed name" {
                ("IPv4HTTPSupport_009F", "Disabled") => Ok(vec![("IPv4HTTPSupport_009F".to_string(), json!("Enabled"))]),
            }
            "spelling family" {
                ("SecurityDeviceSupport_0123", "Disabled") => Ok(vec![("SecurityDeviceSupport_0123".to_string(), json!("Enabled"))]),
                ("SecurityDeviceSupport_0123", "Disable") => Ok(vec![("SecurityDeviceSupport_0123".to_string(), json!("Enable"))]),
                ("SecurityDeviceSupport_0123", "Enable") => Ok(vec![("SecurityDeviceSupport_0123".to_string(), json!("Enable"))]),
            }
            "unknown spelling" {
                ("SecurityDeviceSupport_0123", "Off") => Err(PlatformError::InvalidResponse {
                    message: "SecurityDeviceSupport_0123 reports \"Off\", which is neither spelling".to_string(),
                }),
            }
        );
    }
}
