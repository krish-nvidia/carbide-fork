/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BIOS resources reached from the ComputerSystem exploration selected.

use std::collections::BTreeMap;

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::computer_system::{AttributesUpdate, Bios, BiosUpdate};
use nv_redfish::core::{Bmc, DynamicProperties, EdmPrimitiveType, ModificationResponse};
use nv_redfish::schema::bios::Bios as BiosSchema;
use serde_json::Value;

/// Returns the BIOS resource of the selected ComputerSystem.
pub(crate) async fn selected_bios<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Bios<B>, PlatformError> {
    cx.system()
        .await?
        .bios()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// Returns the pending-settings resource the BIOS advertises through `@Redfish.Settings`.
pub(crate) async fn bios_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
    bios: &Bios<B>,
) -> Result<Bios<B>, PlatformError> {
    bios.settings()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// Attribute values keyed by name, from `(name, value)` pairs.
pub(crate) fn attribute_map<const N: usize>(
    entries: [(&str, Value); N],
) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

/// Stages, on the pending-settings resource, the entries of `attributes` the
/// BIOS would not hold after the next reset, judged by the staged value or,
/// when nothing is staged, the current one. Repeating a stage writes nothing.
pub(crate) async fn stage_bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: &BTreeMap<String, Value>,
) -> Result<DriverOutcome, PlatformError> {
    if attributes.is_empty() {
        return Ok(DriverOutcome::complete());
    }
    let bios = selected_bios(cx).await?;
    let current = bios_attributes(&bios.raw());
    let settings = bios_settings(cx, &bios).await?;
    let pending = bios_attributes(&settings.raw());
    let staged: BTreeMap<String, Value> = attributes
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

/// Writes every entry of `attributes` to the pending-settings resource without
/// reading the BIOS first, for write-only attributes and for BIOSes that cannot
/// be read at the time, such as a BlueField in NIC mode on older firmware.
pub(crate) async fn write_bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: &BTreeMap<String, Value>,
) -> Result<DriverOutcome, PlatformError> {
    update_bios_settings(cx, &bios_update(attributes)?)
        .await
        .map(DriverOutcome::from)
}

/// The BIOS update writing `attributes`.
pub(crate) fn bios_update(
    attributes: &BTreeMap<String, Value>,
) -> Result<BiosUpdate, PlatformError> {
    Ok(BiosUpdate::builder()
        .with_attributes(attributes_update(attributes)?)
        .build())
}

/// Writes `body` to the BIOS pending-settings resource and returns the
/// response, whose task location callers such as Dell track as a job.
pub(crate) async fn update_bios_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
    body: &BiosUpdate,
) -> Result<ModificationResponse<Bios<B>>, PlatformError> {
    let bios = selected_bios(cx).await?;
    bios_settings(cx, &bios)
        .await?
        .update(body)
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

/// Converts the BIOS `Attributes` object into plain JSON values, skipping null attributes.
pub(crate) fn bios_attributes(bios: &BiosSchema) -> BTreeMap<String, Value> {
    bios.attributes
        .as_ref()
        .map(|attributes| {
            attributes
                .dynamic_properties
                .iter()
                .filter_map(|(key, value)| {
                    let value = match value.as_ref()? {
                        EdmPrimitiveType::String(value) => Value::String(value.clone()),
                        EdmPrimitiveType::Bool(value) => Value::Bool(*value),
                        EdmPrimitiveType::Integer(value) => Value::from(*value),
                        EdmPrimitiveType::Decimal(value) => Value::from(*value),
                    };
                    Some((key.clone(), value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Converts plain JSON attribute values into the dynamic BIOS attribute update.
fn attributes_update(
    attributes: &BTreeMap<String, Value>,
) -> Result<AttributesUpdate, PlatformError> {
    Ok(AttributesUpdate::builder()
        .with_dynamic_properties(dynamic_properties(attributes)?)
        .build())
}

/// Converts plain JSON attribute values into dynamic attribute properties;
/// attribute registries allow only primitive values.
pub(crate) fn dynamic_properties(
    attributes: &BTreeMap<String, Value>,
) -> Result<DynamicProperties<EdmPrimitiveType>, PlatformError> {
    attributes
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Value::Null => None,
                Value::String(value) => Some(EdmPrimitiveType::String(value.clone())),
                Value::Bool(value) => Some(EdmPrimitiveType::Bool(*value)),
                Value::Number(number) => Some(
                    number
                        .as_i64()
                        .map(EdmPrimitiveType::Integer)
                        .or_else(|| number.as_f64().map(EdmPrimitiveType::Decimal))
                        .ok_or_else(|| PlatformError::InvalidResponse {
                            message: format!("attribute {key} is out of range: {number}"),
                        })?,
                ),
                Value::Array(_) | Value::Object(_) => {
                    return Err(PlatformError::InvalidResponse {
                        message: format!("attribute {key} is not a primitive value"),
                    });
                }
            };
            Ok((key.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn attribute_updates_serialize_as_the_plain_json_values() {
        let attributes = BTreeMap::from([
            ("Mode".to_string(), json!("Restricted")),
            ("Enabled".to_string(), json!(true)),
            ("Count".to_string(), json!(4)),
        ]);
        let body = bios_update(&attributes).expect("primitive values");
        assert_eq!(
            serde_json::to_value(&body).expect("body serializes"),
            json!({"Attributes": {"Mode": "Restricted", "Enabled": true, "Count": 4}})
        );
        assert!(bios_update(&attribute_map([("List", json!([1]))])).is_err());
    }
}
