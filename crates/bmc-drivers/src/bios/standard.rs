/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::collections::BTreeMap;

use async_trait::async_trait;
use bmc_platform::{Bios, BiosDiff, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc, EntityTypeRef, ODataId};
use serde::Serialize;
use serde_json::{Value, json};

use crate::resources::{
    bios_attributes, bios_settings, patch_bios_attributes, patch_settings, selected_bios,
};

/// How the UEFI administrator password is set.
pub(crate) enum PasswordMethod {
    /// The `#Bios.ChangePassword` action, naming the password `name`.
    Action { name: &'static str },
    /// Write-only attributes on the pending settings (BlueField).
    PendingAttributes,
}

/// How BIOS defaults are restored.
pub(crate) enum ResetMethod {
    /// The `#Bios.ResetBios` action.
    Action,
    /// A write-only boolean attribute on the pending settings (BlueField `ResetEfiVars`).
    PendingAttribute(&'static str),
    /// The UpdateService OEM `ClearNVRAM` action against the host BIOS inventory (Viking).
    ClearNvram,
}

/// Standard BIOS behavior driven by the resource's advertised settings and
/// actions; firmware families vary only in the two methods below.
pub(crate) struct StandardBios {
    pub(crate) password: PasswordMethod,
    pub(crate) reset: ResetMethod,
}

/// DMTF-standard behavior, also used by HPE iLO and Supermicro X13.
pub(crate) static STANDARD_BIOS: StandardBios = StandardBios {
    password: PasswordMethod::Action {
        name: "AdministratorPassword",
    },
    reset: ResetMethod::Action,
};

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

pub(super) async fn current_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<BiosSettings, PlatformError> {
    Ok(BiosSettings {
        attributes: bios_attributes(&selected_bios(cx).await?.raw()),
    })
}

pub(super) async fn pending_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<BiosSettings, PlatformError> {
    let bios = selected_bios(cx).await?;
    let settings = bios_settings(cx, &bios).await?;
    Ok(BiosSettings {
        attributes: bios_attributes(&settings),
    })
}

pub(super) async fn status<B: Bmc>(
    cx: &OpCx<'_, B>,
    expected: &BiosSettings,
) -> Result<BiosStatus, PlatformError> {
    let differences = differences(&pending_settings(cx).await?, expected);
    Ok(BiosStatus {
        is_applied: differences.is_empty(),
        differences,
    })
}

/// Writes every expected attribute to the pending-settings resource unless it
/// already holds every value.
pub(super) async fn apply<B: Bmc>(
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

/// Reverts only the staged attributes that differ, since re-sending every
/// attribute trips read-only rejections on many BMCs.
pub(super) async fn clear_pending<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<DriverOutcome, PlatformError> {
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

pub(super) async fn reset_bios<B>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    B::Error: ActionError,
{
    selected_bios(cx)
        .await?
        .raw()
        .actions
        .as_ref()
        .ok_or(PlatformError::Unsupported)?
        .reset_bios(cx.bmc())
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_bmc_error(error))
}

/// Changes a named UEFI password through the advertised `#Bios.ChangePassword`
/// action; an empty `new_password` clears it.
pub(super) async fn change_password<B>(
    cx: &OpCx<'_, B>,
    password_name: &str,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    B::Error: ActionError,
{
    selected_bios(cx)
        .await?
        .raw()
        .actions
        .as_ref()
        .ok_or(PlatformError::Unsupported)?
        .change_password(
            cx.bmc(),
            password_name.to_string(),
            Some(current_password.to_string()),
            new_password.to_string(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_bmc_error(error))
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

/// Clears the host BIOS NVRAM through the NVIDIA UpdateService OEM action.
async fn clear_nvram<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let service = cx
        .service_root()
        .update_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let raw = service.raw();
    let target = ODataId::from(format!(
        "{}/Actions/Oem/NvidiaUpdateService.ClearNVRAM",
        raw.odata_id()
    ));
    let host_bios = format!("{}/FirmwareInventory/HostBIOS_0", raw.odata_id());
    cx.post(&target, &json!({"Targets": [host_bios]})).await
}

impl StandardBios {
    async fn set_password<B>(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError>
    where
        B: Bmc,
        B::Error: ActionError,
    {
        match self.password {
            PasswordMethod::Action { name } => {
                change_password(cx, name, current_password, new_password).await
            }
            PasswordMethod::PendingAttributes => {
                set_password_attributes(cx, current_password, new_password).await
            }
        }
    }
}

#[async_trait]
impl<B> Bios<B> for StandardBios
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn current(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        current_settings(cx).await
    }

    async fn pending(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        pending_settings(cx).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        status(cx, expected).await
    }

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        apply(cx, expected).await
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        match self.reset {
            ResetMethod::Action => reset_bios(cx).await,
            ResetMethod::PendingAttribute(key) => {
                patch_bios_attributes(cx, json!({key: true})).await
            }
            ResetMethod::ClearNvram => clear_nvram(cx).await,
        }
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
        self.set_password(cx, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.set_password(cx, current_password, "").await
    }
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
