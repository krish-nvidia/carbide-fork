/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BIOS resources reached from the ComputerSystem exploration selected.

use std::collections::BTreeMap;

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::computer_system::{AttributesUpdate, Bios, BiosUpdate};
use nv_redfish::core::{
    Bmc, DynamicProperties, EdmPrimitiveType, EntityTypeRef, ModificationResponse,
};
use nv_redfish::schema::bios::Bios as BiosSchema;
use serde::Serialize;
use serde_json::Value;

use crate::update;

/// Returns the BIOS resource of the selected ComputerSystem.
pub(crate) async fn selected_bios<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Bios<B>, PlatformError> {
    cx.system()?
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

/// Stages `attributes` on an already fetched pending-settings resource.
pub(crate) async fn update_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
    settings: &Bios<B>,
    attributes: &BTreeMap<String, Value>,
) -> Result<DriverOutcome, PlatformError> {
    let body = BiosUpdate::builder()
        .with_attributes(attributes_update(attributes)?)
        .build();
    update::apply(cx, settings.raw().as_ref(), &body, settings.update(&body)).await
}

/// Stages BIOS attribute values, given as a JSON object, through the
/// pending-settings resource.
pub(crate) async fn patch_bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: Value,
) -> Result<DriverOutcome, PlatformError> {
    let Value::Object(attributes) = attributes else {
        return Err(PlatformError::InvalidResponse {
            message: "BIOS attributes must be a JSON object".to_string(),
        });
    };
    let bios = selected_bios(cx).await?;
    let settings = bios_settings(cx, &bios).await?;
    update_settings(cx, &settings, &attributes.into_iter().collect()).await
}

/// Writes `payload` to the BIOS pending-settings resource and returns the raw
/// response, for bodies `BiosUpdate` cannot carry, such as Dell's
/// `@Redfish.SettingsApplyTime`.
pub(crate) async fn patch_bios_settings<B, T>(
    cx: &OpCx<'_, B>,
    payload: &T,
) -> Result<ModificationResponse<Value>, PlatformError>
where
    B: Bmc,
    T: Serialize + Send + Sync,
{
    let bios = selected_bios(cx).await?;
    let settings = bios_settings(cx, &bios).await?.raw();
    cx.patch_id(settings.odata_id(), settings.etag(), payload)
        .await
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
                            message: format!("BIOS attribute {key} is out of range: {number}"),
                        })?,
                ),
                Value::Array(_) | Value::Object(_) => {
                    return Err(PlatformError::InvalidResponse {
                        message: format!("BIOS attribute {key} is not a primitive value"),
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
        let body = BiosUpdate::builder()
            .with_attributes(attributes_update(&attributes).expect("primitive values"))
            .build();
        assert_eq!(
            serde_json::to_value(&body).expect("body serializes"),
            json!({"Attributes": {"Mode": "Restricted", "Enabled": true, "Count": 4}})
        );
        assert!(attributes_update(&BTreeMap::from([("List".to_string(), json!([1]))])).is_err());
    }
}
