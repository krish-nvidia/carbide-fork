/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::bios::attributes::nvidia::bluefield as table;
use crate::bios::standard::{self, StandardBios, differences};
use crate::resources::patch_bios_attributes;

/// NVIDIA BlueField: the DPU BIOS exposes no `ResetBios` or `ChangePassword`
/// actions; both are write-only attributes on the pending settings. The
/// pending settings list nothing once staged values take effect on the next
/// DPU boot, so status is read from the current attributes.
///
/// BMC 24.10 dropped the spaces from some attribute names, so expected values
/// are matched to whichever spelling the BIOS reports. An attribute missing
/// under both spellings means the UEFI has not finished POST.
pub(crate) struct BlueFieldBios;

/// Attributes reported either without or with spaces, depending on firmware.
const RENAMED: [(&str, &str); 2] = [
    ("HostPrivilegeLevel", "Host Privilege Level"),
    ("InternalCPUModel", "Internal CPU Model"),
];

/// `expected` with each renamed attribute under the spelling `current`
/// reports. When `current` reports neither, the unspaced spelling is used
/// unless `require_reported` makes that an error.
fn reported_spellings(
    current: &BiosSettings,
    expected: &BiosSettings,
    require_reported: bool,
) -> Result<BiosSettings, PlatformError> {
    let mut attributes = expected.attributes.clone();
    for (unspaced, spaced) in RENAMED {
        let Some(value) = attributes
            .remove(unspaced)
            .into_iter()
            .chain(attributes.remove(spaced))
            .next()
        else {
            continue;
        };
        let name = if current.attributes.contains_key(spaced) {
            spaced
        } else if current.attributes.contains_key(unspaced) || !require_reported {
            unspaced
        } else {
            return Err(PlatformError::InvalidResponse {
                message: format!("BlueField BIOS does not report {unspaced}"),
            });
        };
        attributes.insert(name.to_string(), value);
    }
    Ok(BiosSettings { attributes })
}

async fn set_password_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError> {
    patch_bios_attributes(
        cx,
        json!({
            "CurrentUefiPassword": current_password,
            "UefiPassword": new_password,
        }),
    )
    .await
}

#[async_trait]
impl<B: Bmc> Bios<B> for BlueFieldBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn expected(
        &self,
        cx: &OpCx<'_, B>,
        overlay: &BiosSettings,
    ) -> Result<BiosSettings, PlatformError> {
        standard::expected(cx, table::ATTRIBUTES, overlay).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let current = self.current(cx).await?;
        let differences = differences(&current, &reported_spellings(&current, expected, true)?);
        Ok(BiosStatus {
            is_applied: differences.is_empty(),
            differences,
        })
    }

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = self.current(cx).await?;
        self.standard()
            .apply(cx, &reported_spellings(&current, expected, false)?)
            .await
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        patch_bios_attributes(cx, json!({"ResetEfiVars": true})).await
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        set_password_attributes(cx, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        set_password_attributes(cx, current_password, "").await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::test_support::Fixture;

    #[tokio::test]
    async fn status_compares_the_current_attributes_not_the_empty_pending_object() {
        const BIOS: &str = "/redfish/v1/Systems/Bluefield/Bios";
        const SETTINGS: &str = "/redfish/v1/Systems/Bluefield/Bios/Settings";
        let bmc = Fixture::new("Nvidia", "BlueField-3 DPU", "Bluefield", "Bluefield_BMC")
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
                    "Attributes": {"HostPrivilegeLevel": "Restricted", "InternalCPUModel": "Separated"},
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
            .await;
        let cx = bmc.cx().await;
        let expected = BiosSettings {
            attributes: BTreeMap::from([
                ("HostPrivilegeLevel".to_string(), json!("Restricted")),
                ("Host Privilege Level".to_string(), json!("Restricted")),
                ("InternalCPUModel".to_string(), json!("Embedded")),
                ("Internal CPU Model".to_string(), json!("Embedded")),
            ]),
        };

        let status = BlueFieldBios
            .status(&cx, &expected)
            .await
            .expect("status reads");
        assert!(!status.is_applied);
        assert_eq!(
            status
                .differences
                .iter()
                .map(|difference| difference.key.as_str())
                .collect::<Vec<_>>(),
            ["InternalCPUModel"]
        );
    }
}
