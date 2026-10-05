/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::bios::attributes::{BiosAttribute, desired_settings};
use crate::bios::standard::StandardBios;
use crate::bios::support::{compare, current_settings, settings, stage, with_profile};
use crate::resources::{attribute_map, patch_bios_attributes};

/// NVIDIA BlueField: the DPU BIOS exposes no `ResetBios` or `ChangePassword`
/// actions; both are write-only attributes on the pending settings.
///
/// BMC 24.10 dropped the spaces from some attribute names, so expected values
/// take whichever spelling the BIOS reports. Until the UEFI finishes POST the
/// BIOS reports neither, so the attributes show as missing.
pub(crate) struct BlueFieldBios;

/// Attributes reported either without or with spaces, depending on firmware.
const RENAMED: [(&str, &str); 2] = [
    ("HostPrivilegeLevel", "Host Privilege Level"),
    ("InternalCPUModel", "Internal CPU Model"),
];

/// Declared under their unspaced names; [`reported_spellings`] renames them.
const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("HostPrivilegeLevel", "Restricted").required(),
    BiosAttribute::string("InternalCPUModel", "Embedded").required(),
];

/// `settings` with each renamed attribute under its unspaced name.
fn unspaced(mut settings: BiosSettings) -> BiosSettings {
    for (unspaced, spaced) in RENAMED {
        if let Some(value) = settings.attributes.remove(spaced) {
            settings.attributes.insert(unspaced.to_string(), value);
        }
    }
    settings
}

/// `expected` with each renamed attribute under the spaced name when `current`
/// reports that spelling.
fn reported_spellings(current: &BiosSettings, mut expected: BiosSettings) -> BiosSettings {
    for (unspaced, spaced) in RENAMED {
        if current.attributes.contains_key(spaced)
            && let Some(value) = expected.attributes.remove(unspaced)
        {
            expected.attributes.insert(spaced.to_string(), value);
        }
    }
    expected
}

/// The table and `profile`, normalized to unspaced names, then renamed to the
/// spelling `current` reports.
fn expected_settings(current: &BiosSettings, profile: &BiosSettings) -> BiosSettings {
    let expected = with_profile(
        desired_settings(ATTRIBUTES, &unspaced(current.clone())),
        &unspaced(profile.clone()),
    );
    reported_spellings(current, expected)
}

#[async_trait]
impl<B: Bmc> Bios<B> for BlueFieldBios
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
    ) -> Result<DriverOutcome, PlatformError> {
        let current = current_settings(cx).await?;
        stage(cx, &expected_settings(&current, profile)).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let current = current_settings(cx).await?;
        Ok(compare(&current, &expected_settings(&current, profile)))
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        stage(cx, &settings([("ResetEfiVars", json!(true))])).await
    }

    /// Written unconditionally: the password attributes are write-only, so a
    /// value that happens to read back must still be sent.
    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        patch_bios_attributes(
            cx,
            &attribute_map([
                ("CurrentUefiPassword", json!(current_password)),
                ("UefiPassword", json!(new_password)),
            ]),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, FixtureBmc};

    const BIOS: &str = "/redfish/v1/Systems/Bluefield/Bios";
    const SETTINGS: &str = "/redfish/v1/Systems/Bluefield/Bios/Settings";

    async fn bluefield(attributes: serde_json::Value) -> FixtureBmc {
        Fixture::new("Nvidia", "BlueField-3 DPU", "Bluefield", "Bluefield_BMC")
            .document(
                "/redfish/v1/Systems/Bluefield",
                json!({
                    "@odata.id": "/redfish/v1/Systems/Bluefield",
                    "Id": "Bluefield",
                    "Name": "Bluefield",
                    "Bios": {"@odata.id": BIOS},
                }),
            )
            .document(
                BIOS,
                json!({
                    "@odata.id": BIOS,
                    "@Redfish.Settings": {"SettingsObject": {"@odata.id": SETTINGS}},
                    "Id": "BIOS",
                    "Name": "BIOS Configuration Current Settings",
                    "Attributes": attributes,
                }),
            )
            .document(
                SETTINGS,
                json!({
                    "@odata.id": SETTINGS,
                    "Id": "BIOS_Settings",
                    "Name": "BIOS Configuration",
                    "Attributes": {},
                }),
            )
            .build()
            .await
    }

    #[tokio::test]
    async fn status_follows_the_reported_spelling_and_flags_a_bios_before_post() {
        struct Case {
            scenario: &'static str,
            reported: serde_json::Value,
            differences: Vec<&'static str>,
        }
        let cases = [
            Case {
                scenario: "unspaced firmware with one setting wrong",
                reported: json!({"HostPrivilegeLevel": "Restricted", "InternalCPUModel": "Separated"}),
                differences: vec!["InternalCPUModel"],
            },
            Case {
                scenario: "spaced firmware already set up",
                reported: json!({"Host Privilege Level": "Restricted", "Internal CPU Model": "Embedded"}),
                differences: vec![],
            },
            Case {
                scenario: "UEFI has not finished POST",
                reported: json!({}),
                differences: vec!["HostPrivilegeLevel", "InternalCPUModel"],
            },
        ];

        for case in cases {
            let bmc = bluefield(case.reported).await;
            let cx = bmc.cx().await;
            let status = BlueFieldBios
                .status(&cx, &BiosSettings::default())
                .await
                .expect("status reads");
            assert_eq!(
                status
                    .differences
                    .iter()
                    .map(|difference| difference.key.as_str())
                    .collect::<Vec<_>>(),
                case.differences,
                "{}",
                case.scenario
            );
        }
    }
}
