/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish BIOS configuration.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bmc_platform::{Bios, BiosDiff, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{Bmc, EntityTypeRef, ModificationResponse};
use nv_redfish::schema::bios::{Bios as BiosSchema, BiosChangePasswordAction, BiosResetBiosAction};
use serde::Serialize;
use serde_json::Value;

use crate::resources::{bios_attributes, bios_settings, selected_bios};

/// DMTF names the UEFI administrator password `AdministratorPassword`.
const UEFI_PASSWORD_NAME: &str = "AdministratorPassword";

/// DMTF-standard BIOS behavior, also used by HPE iLO and Supermicro X13.
pub(crate) struct StandardBios;

#[async_trait]
impl<B: Bmc> Bios<B> for StandardBios {
    fn standard(&self) -> &dyn Bios<B> {
        self
    }

    async fn current(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        Ok(BiosSettings {
            attributes: bios_attributes(&selected_bios(cx).await?.raw()),
        })
    }

    async fn pending(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        pending(cx).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let differences = differences(&pending(cx).await?, expected);
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

/// The attributes staged on the pending-settings resource.
async fn pending<B: Bmc>(cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
    let bios = selected_bios(cx).await?;
    let settings = bios_settings(cx, &bios).await?;
    Ok(BiosSettings {
        attributes: bios_attributes(&settings),
    })
}

/// Writes every expected attribute to the pending-settings resource unless it
/// already holds every value.
async fn apply<B: Bmc>(
    cx: &OpCx<'_, B>,
    expected: &BiosSettings,
) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?;
    let settings = bios_settings(cx, &bios).await?;
    let pending = BiosSettings {
        attributes: bios_attributes(&settings),
    };
    if differences(&pending, expected).is_empty() {
        return Ok(DriverOutcome::complete());
    }
    patch_settings(
        cx,
        &settings,
        &AttributesPayload {
            attributes: &expected.attributes,
        },
    )
    .await
    .map(DriverOutcome::from)
}

/// Restores BIOS defaults through the advertised `Bios.ResetBios` action.
async fn reset<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?.raw();
    let action = bios
        .actions
        .as_ref()
        .and_then(|actions| actions.reset_bios.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    cx.action(action, &BiosResetBiosAction {}).await
}

/// Reverts only the staged attributes that differ, since re-sending every
/// attribute trips read-only rejections on many BMCs.
async fn clear_pending<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?;
    let current = bios_attributes(&bios.raw());
    let settings = bios_settings(cx, &bios).await?;
    let reverted: BTreeMap<String, Value> = bios_attributes(&settings)
        .into_iter()
        .filter_map(|(key, pending)| {
            let current = current.get(&key)?;
            (*current != pending).then(|| (key, current.clone()))
        })
        .collect();
    if reverted.is_empty() {
        return Ok(DriverOutcome::complete());
    }
    patch_settings(
        cx,
        &settings,
        &AttributesPayload {
            attributes: &reverted,
        },
    )
    .await
    .map(DriverOutcome::from)
}

/// Changes the UEFI password named `password_name` through the advertised
/// `Bios.ChangePassword` action; an empty `new_password` clears it.
pub(super) async fn change_password<B: Bmc>(
    cx: &OpCx<'_, B>,
    password_name: &str,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?.raw();
    let action = bios
        .actions
        .as_ref()
        .and_then(|actions| actions.change_password.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    cx.action(
        action,
        &BiosChangePasswordAction {
            password_name: password_name.to_string(),
            old_password: Some(current_password.to_string()),
            new_password: new_password.to_string(),
        },
    )
    .await
}

/// Writes `payload` to an already fetched pending-settings resource with its ETag.
async fn patch_settings<B, T>(
    cx: &OpCx<'_, B>,
    settings: &BiosSchema,
    payload: &T,
) -> Result<ModificationResponse<Value>, PlatformError>
where
    B: Bmc,
    T: Serialize + Send + Sync,
{
    cx.patch_id(settings.odata_id(), settings.etag(), payload)
        .await
}

#[derive(Serialize)]
struct AttributesPayload<'a> {
    #[serde(rename = "Attributes")]
    attributes: &'a BTreeMap<String, Value>,
}

fn differences(actual: &BiosSettings, expected: &BiosSettings) -> Vec<BiosDiff> {
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
    use serde_json::json;

    use super::*;

    #[test]
    fn differences_report_only_missing_or_changed_attributes() {
        let actual = BiosSettings {
            attributes: BTreeMap::from([
                ("same".to_string(), json!("on")),
                ("changed".to_string(), json!(1)),
            ]),
        };
        let expected = BiosSettings {
            attributes: BTreeMap::from([
                ("same".to_string(), json!("on")),
                ("changed".to_string(), json!(2)),
                ("missing".to_string(), json!(true)),
            ]),
        };
        assert_eq!(
            differences(&actual, &expected)
                .iter()
                .map(|difference| difference.key.as_str())
                .collect::<Vec<_>>(),
            ["changed", "missing"]
        );
    }
}
