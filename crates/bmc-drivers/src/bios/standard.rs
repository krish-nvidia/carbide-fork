/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish BIOS configuration.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bmc_platform::{Bios, BiosDiff, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::Value;

use crate::bios::attributes::{BiosAttribute, desired_settings};
use crate::resources::{bios_attributes, bios_settings, selected_bios, update_settings};

/// DMTF names the UEFI administrator password `AdministratorPassword`.
const UEFI_PASSWORD_NAME: &str = "AdministratorPassword";

/// DMTF-standard BIOS behavior, also used by HPE iLO and Supermicro X13.
pub(crate) struct StandardBios;

#[async_trait]
impl<B: Bmc> Bios<B> for StandardBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        self
    }

    /// Redfish has no standard TPM clear.
    async fn clear_tpm(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn current(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        current(cx).await
    }

    async fn pending(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        pending(cx).await
    }

    /// A platform without its own settings expects only the caller's profile.
    async fn expected(
        &self,
        _cx: &OpCx<'_, B>,
        overlay: &BiosSettings,
    ) -> Result<BiosSettings, PlatformError> {
        Ok(overlay.clone())
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let differences = differences(&current(cx).await?, expected);
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
        apply(cx, expected).await
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        reset(cx).await
    }

    async fn clear_pending(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        clear_pending(cx).await
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password(cx, UEFI_PASSWORD_NAME, current_password, "").await
    }
}

/// The attributes the BIOS currently runs with.
pub(super) async fn current<B: Bmc>(cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
    Ok(BiosSettings {
        attributes: bios_attributes(&selected_bios(cx).await?.raw()),
    })
}

/// Pending-settings entries that differ from the current value; many BMCs
/// echo every attribute on the settings resource.
async fn pending<B: Bmc>(cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
    let bios = selected_bios(cx).await?;
    let current = bios_attributes(&bios.raw());
    let settings = bios_settings(cx, &bios).await?;
    Ok(BiosSettings {
        attributes: bios_attributes(&settings.raw())
            .into_iter()
            .filter(|(key, pending)| current.get(key) != Some(pending))
            .collect(),
    })
}

/// Stages the expected attributes the BIOS would not hold after the next
/// reset, judged by the staged value or, when nothing is staged, the current
/// one.
async fn apply<B: Bmc>(
    cx: &OpCx<'_, B>,
    expected: &BiosSettings,
) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?;
    let current = bios_attributes(&bios.raw());
    let settings = bios_settings(cx, &bios).await?;
    let pending = bios_attributes(&settings.raw());
    let staged: BTreeMap<String, Value> = expected
        .attributes
        .iter()
        .filter(|(key, value)| pending.get(*key).or_else(|| current.get(*key)) != Some(value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if staged.is_empty() {
        return Ok(DriverOutcome::complete());
    }
    update_settings(cx, &settings, &staged).await
}

/// The settings `attributes` call for on the current BIOS, with the caller's
/// `overlay` taking precedence.
pub(super) fn resolve(
    attributes: &[BiosAttribute],
    current: &BiosSettings,
    overlay: &BiosSettings,
) -> Result<BiosSettings, PlatformError> {
    let mut expected =
        desired_settings(attributes, current).map_err(|error| PlatformError::InvalidResponse {
            message: error.to_string(),
        })?;
    expected.attributes.extend(
        overlay
            .attributes
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    Ok(expected)
}

/// [`resolve`] against the BIOS's current attributes.
pub(super) async fn expected<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: &[BiosAttribute],
    overlay: &BiosSettings,
) -> Result<BiosSettings, PlatformError> {
    resolve(attributes, &current(cx).await?, overlay)
}

/// Restores BIOS defaults through the advertised `Bios.ResetBios` action.
async fn reset<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    selected_bios(cx)
        .await?
        .reset()
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Reverts only the staged attributes that differ, since re-sending every
/// attribute trips read-only rejections on many BMCs.
async fn clear_pending<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?;
    let current = bios_attributes(&bios.raw());
    let settings = bios_settings(cx, &bios).await?;
    let reverted: BTreeMap<String, Value> = bios_attributes(&settings.raw())
        .into_iter()
        .filter_map(|(key, pending)| {
            let current = current.get(&key)?;
            (*current != pending).then(|| (key, current.clone()))
        })
        .collect();
    if reverted.is_empty() {
        return Ok(DriverOutcome::complete());
    }
    update_settings(cx, &settings, &reverted).await
}

/// Changes the UEFI password named `password_name` through the advertised
/// `Bios.ChangePassword` action; an empty `new_password` clears it.
pub(super) async fn change_password<B: Bmc>(
    cx: &OpCx<'_, B>,
    password_name: &str,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    selected_bios(cx)
        .await?
        .change_password(
            password_name.to_string(),
            Some(current_password.to_string()),
            new_password.to_string(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

pub(super) fn differences(actual: &BiosSettings, expected: &BiosSettings) -> Vec<BiosDiff> {
    expected
        .attributes
        .iter()
        .filter_map(|(key, expected)| {
            let actual = actual.attributes.get(key);
            (actual != Some(expected)).then(|| BiosDiff {
                key: key.clone(),
                expected: expected.clone(),
                actual: actual.cloned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use axum::http::Method;
    use serde_json::json;

    use super::*;
    use crate::test_support::{Fixture, body, path};

    const SYSTEM: &str = "/redfish/v1/Systems/1";
    const BIOS: &str = "/redfish/v1/Systems/1/Bios";
    const SETTINGS: &str = "/redfish/v1/Systems/1/Bios/Settings";

    fn settings(attributes: serde_json::Value) -> BiosSettings {
        serde_json::from_value(json!({ "attributes": attributes })).expect("settings")
    }

    #[tokio::test]
    async fn status_reads_current_values_and_apply_stages_only_what_the_next_reset_lacks() {
        let bmc = Fixture::new("Contoso", "Server", "1", "1")
            .document(
                SYSTEM,
                json!({"@odata.id": SYSTEM, "Id": "1", "Name": "System", "Bios": {"@odata.id": BIOS}}),
            )
            .document(
                BIOS,
                json!({
                    "@odata.id": BIOS,
                    "@Redfish.Settings": {"SettingsObject": {"@odata.id": SETTINGS}},
                    "Id": "Bios",
                    "Name": "BIOS",
                    "Attributes": {"Kept": "on", "Staged": "old", "Changed": "old", "Echoed": "x"},
                }),
            )
            .document(
                SETTINGS,
                json!({
                    "@odata.id": SETTINGS,
                    "Id": "Settings",
                    "Name": "BIOS Settings",
                    "Attributes": {"Staged": "new", "Echoed": "x"},
                }),
            )
            .build()
            .await;
        let cx = bmc.cx().await;
        let expected = settings(json!({
            "Kept": "on",
            "Staged": "new",
            "Changed": "new",
            "Missing": true,
        }));

        let status = StandardBios.status(&cx, &expected).await.expect("status");
        assert_eq!(
            status
                .differences
                .iter()
                .map(|difference| difference.key.as_str())
                .collect::<Vec<_>>(),
            ["Changed", "Missing", "Staged"]
        );
        assert_eq!(
            StandardBios.pending(&cx).await,
            Ok(settings(json!({"Staged": "new"})))
        );
        assert_eq!(
            StandardBios.apply(&cx, &expected).await,
            Ok(DriverOutcome::complete())
        );
        let writes = bmc.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].method, Method::PATCH);
        assert_eq!(path(&writes[0]), SETTINGS);
        assert_eq!(
            body(&writes[0]),
            json!({"Attributes": {"Changed": "new", "Missing": true}})
        );
    }

    #[test]
    fn resolve_writes_only_reported_attributes_and_the_profile_wins() {
        let table = [
            BiosAttribute::string("Virtualization", "Enabled"),
            BiosAttribute::string("AbsentOnThisModel", "Enabled"),
        ];
        assert_eq!(
            resolve(
                &table,
                &settings(json!({"Virtualization": "Disabled", "Other": "x"})),
                &settings(json!({"Virtualization": "Profile", "PowerProfile": "Performance"})),
            ),
            Ok(settings(json!({
                "Virtualization": "Profile",
                "PowerProfile": "Performance",
            })))
        );
    }
}
