/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Firmware mechanics shared by several vendor drivers.

use bmc_platform::{OpCx, PlatformError};
use nv_redfish::core::{Bmc, EntityTypeRef};

use crate::firmware::standard::{advertised_multipart_uri, update_service};

/// The advertised `MultipartHttpPushUri`, or `fallback` for firmware that
/// implements multipart push without advertising it.
pub(super) async fn upload_uri<B: Bmc>(
    cx: &OpCx<'_, B>,
    fallback: &str,
) -> Result<String, PlatformError> {
    let service = update_service(cx).await?;
    Ok(advertised_multipart_uri(&service).unwrap_or_else(|| fallback.to_string()))
}

/// The `@odata.id` of each firmware inventory entry in `ids`, for update
/// targets; `Unsupported` when the BMC lists one of them under no such id.
pub(super) async fn inventory_targets<B: Bmc>(
    cx: &OpCx<'_, B>,
    ids: &[&str],
) -> Result<Vec<String>, PlatformError> {
    let inventories = update_service(cx)
        .await?
        .firmware_inventories()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    ids.iter()
        .map(|id| {
            inventories
                .iter()
                .map(|inventory| inventory.raw())
                .find(|inventory| inventory.id == *id)
                .map(|inventory| inventory.odata_id().to_string())
                .ok_or(PlatformError::Unsupported)
        })
        .collect()
}

/// The `@odata.id` of the chassis `id`, for update targets.
pub(super) async fn chassis_target<B: Bmc>(
    cx: &OpCx<'_, B>,
    id: &str,
) -> Result<String, PlatformError> {
    cx.service_root()
        .chassis()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .iter()
        .map(|chassis| chassis.raw())
        .find(|chassis| chassis.id == id)
        .map(|chassis| chassis.odata_id().to_string())
        .ok_or(PlatformError::Unsupported)
}
