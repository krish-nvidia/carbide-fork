/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BIOS resources reached from the ComputerSystem exploration selected.

use std::collections::BTreeMap;
use std::sync::Arc;

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::computer_system::Bios;
use nv_redfish::core::{
    Bmc, EdmPrimitiveType, EntityTypeRef, ModificationResponse, RedfishSettings,
};
use nv_redfish::schema::bios::Bios as BiosSchema;
use serde::Serialize;
use serde_json::Value;

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
) -> Result<Arc<BiosSchema>, PlatformError> {
    let settings = bios
        .raw()
        .settings_object()
        .ok_or(PlatformError::Unsupported)?;
    cx.bmc()
        .get::<BiosSchema>(settings.id())
        .await
        .map_err(|error| cx.map_bmc_error(error))
}

/// Writes `payload` to the BIOS pending-settings resource and returns the raw
/// response, for callers that must tell an entity from a task.
pub(crate) async fn patch_bios_settings<B, T>(
    cx: &OpCx<'_, B>,
    payload: &T,
) -> Result<ModificationResponse<Value>, PlatformError>
where
    B: Bmc,
    T: Serialize + Send + Sync,
{
    let bios = selected_bios(cx).await?;
    let settings = bios_settings(cx, &bios).await?;
    patch_settings(cx, &settings, payload).await
}

/// Stages BIOS attribute values through the pending-settings resource.
pub(crate) async fn patch_bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: Value,
) -> Result<DriverOutcome, PlatformError> {
    patch_bios_settings(cx, &serde_json::json!({"Attributes": attributes}))
        .await
        .map(DriverOutcome::from)
}

/// Writes `payload` to an already fetched pending-settings resource with its ETag.
pub(crate) async fn patch_settings<B, T>(
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
