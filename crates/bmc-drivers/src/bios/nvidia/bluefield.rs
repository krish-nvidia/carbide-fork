/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::bios::standard::{StandardBios, differences};
use crate::resources::patch_bios_attributes;

/// NVIDIA BlueField: the DPU BIOS exposes no `ResetBios` or `ChangePassword`
/// actions; both are write-only attributes on the pending settings. The
/// pending settings list nothing once staged values take effect on the next
/// DPU boot, so status is read from the current attributes.
pub(crate) struct BlueFieldBios;

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

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let differences = differences(&self.current(cx).await?, expected);
        Ok(BiosStatus {
            is_applied: differences.is_empty(),
            differences,
        })
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
                ("InternalCPUModel".to_string(), json!("Embedded")),
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
