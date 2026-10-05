/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BIOS operations shared by the BIOS drivers.
//!
//! A driver reads the BIOS's [`current_settings`], works out the settings it
//! expects from them, then either [`compare`]s or [`stage`]s those.

use std::collections::BTreeMap;

use bmc_platform::{BiosDiff, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::Value;

use crate::bios::attributes::{BiosAttribute, desired_settings};
use crate::resources::{attribute_map, bios_attributes, bios_settings, bios_update, selected_bios};

/// The attributes the BIOS currently runs with.
pub(super) async fn current_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<BiosSettings, PlatformError> {
    Ok(BiosSettings {
        attributes: bios_attributes(&selected_bios(cx).await?.raw()),
    })
}

/// The settings `attributes` call for on a BIOS reporting `current`, with the
/// caller's `profile` taking precedence.
pub(super) fn expected(
    attributes: &[BiosAttribute],
    current: &BiosSettings,
    profile: &BiosSettings,
) -> BiosSettings {
    with_profile(desired_settings(attributes, current), profile)
}

/// `settings` with every entry of the caller's `profile` added, replacing any
/// entry for the same attribute.
pub(super) fn with_profile(mut settings: BiosSettings, profile: &BiosSettings) -> BiosSettings {
    settings.attributes.extend(
        profile
            .attributes
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    settings
}

/// Settings holding exactly `entries`.
pub(super) fn settings<const N: usize>(entries: [(&str, Value); N]) -> BiosSettings {
    BiosSettings {
        attributes: attribute_map(entries),
    }
}

/// Whether `current` holds `expected`, with every expected attribute whose
/// current value differs.
pub(super) fn compare(current: &BiosSettings, expected: &BiosSettings) -> BiosStatus {
    let differences: Vec<BiosDiff> = expected
        .attributes
        .iter()
        .filter_map(|(key, expected)| {
            let actual = current.attributes.get(key);
            (actual != Some(expected)).then(|| BiosDiff {
                key: key.clone(),
                expected: expected.clone(),
                actual: actual.cloned(),
            })
        })
        .collect();
    BiosStatus {
        is_applied: differences.is_empty(),
        differences,
    }
}

/// Stages, on the pending-settings resource, the entries of `writes` the BIOS
/// would not hold after the next reset, judged by the staged value or, when
/// nothing is staged, the current one.
pub(super) async fn stage<B: Bmc>(
    cx: &OpCx<'_, B>,
    writes: &BiosSettings,
) -> Result<DriverOutcome, PlatformError> {
    if writes.attributes.is_empty() {
        return Ok(DriverOutcome::complete());
    }
    let bios = selected_bios(cx).await?;
    let current = bios_attributes(&bios.raw());
    let settings = bios_settings(cx, &bios).await?;
    let pending = bios_attributes(&settings.raw());
    let staged: BTreeMap<String, Value> = writes
        .attributes
        .iter()
        .filter(|(key, value)| pending.get(*key).or_else(|| current.get(*key)) != Some(value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if staged.is_empty() {
        return Ok(DriverOutcome::complete());
    }
    settings
        .update(&bios_update(&staged)?)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Whether the current BIOS holds `attribute`'s expected value; `None` when
/// the BIOS does not report the attribute.
pub(super) async fn attribute_holds<B: Bmc>(
    cx: &OpCx<'_, B>,
    attribute: BiosAttribute,
) -> Result<Option<bool>, PlatformError> {
    Ok(current_settings(cx)
        .await?
        .attributes
        .get(attribute.name)
        .map(|value| attribute.value.matches(value)))
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
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn settings(attributes: serde_json::Value) -> BiosSettings {
        serde_json::from_value(json!({ "attributes": attributes })).expect("settings")
    }

    #[test]
    fn profile_wins_over_the_platform_expectations() {
        let table = [
            BiosAttribute::string("Virtualization", "Enabled"),
            BiosAttribute::string("AbsentOnThisModel", "Enabled"),
            BiosAttribute::string("ExpectedEverywhere", "Enabled").required(),
        ];
        assert_eq!(
            expected(
                &table,
                &settings(json!({"Virtualization": "Disabled", "Other": "x"})),
                &settings(json!({"Virtualization": "Profile", "PowerProfile": "Performance"})),
            ),
            settings(json!({
                "Virtualization": "Profile",
                "ExpectedEverywhere": "Enabled",
                "PowerProfile": "Performance",
            }))
        );
    }
}
