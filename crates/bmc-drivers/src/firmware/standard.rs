/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish firmware operations.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Firmware, OpCx, PlatformError};
use nv_redfish::core::{Bmc, MultipartUpdateRequest, UploadReader};
use nv_redfish::schema::software_inventory::SoftwareInventory;
use nv_redfish::schema::update_service::UpdateServiceSimpleUpdateAction;
use nv_redfish::update_service::{MultipartUpdateParameters, UpdateService};
use serde::Serialize;
use serde_json::Value;

/// The multipart request as the [`Firmware`] contract receives it.
pub(super) type UploadRequest<'a> =
    MultipartUpdateRequest<'a, Pin<Box<dyn UploadReader>>, MultipartUpdateParameters>;

/// Standard Redfish inventory, SimpleUpdate, and multipart upload.
pub(crate) struct StandardFirmware;

#[async_trait]
impl<B: Bmc> Firmware<B> for StandardFirmware {
    fn standard(&self) -> &dyn Firmware<B> {
        self
    }

    async fn inventory(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<Arc<SoftwareInventory>>, PlatformError> {
        let inventories = update_service(cx)
            .await?
            .firmware_inventories()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?;
        Ok(inventories
            .into_iter()
            .map(|inventory| inventory.raw())
            .collect())
    }

    /// Uploads through the advertised `MultipartHttpPushUri`.
    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        request: UploadRequest<'_>,
    ) -> Result<DriverOutcome, PlatformError> {
        let service = update_service(cx).await?;
        let uri = advertised_multipart_uri(&service).ok_or(PlatformError::Unsupported)?;
        upload(cx, request, &uri).await
    }

    /// Runs the advertised `UpdateService.SimpleUpdate` action.
    async fn simple_update(
        &self,
        cx: &OpCx<'_, B>,
        request: &UpdateServiceSimpleUpdateAction,
    ) -> Result<DriverOutcome, PlatformError> {
        let service = update_service(cx).await?.raw();
        let action = service
            .actions
            .as_ref()
            .and_then(|actions| actions.simple_update.as_ref())
            .ok_or(PlatformError::Unsupported)?;
        cx.action(action, request).await
    }
}

/// The service root's update service.
pub(super) async fn update_service<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<UpdateService<B>, PlatformError> {
    cx.service_root()
        .update_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// The non-empty `MultipartHttpPushUri` the update service advertises, if any.
pub(super) fn advertised_multipart_uri<B: Bmc>(service: &UpdateService<B>) -> Option<String> {
    service
        .raw()
        .multipart_http_push_uri
        .clone()
        .filter(|uri| !uri.trim().is_empty())
}

/// Uploads to `uri`; a 404 means the BMC does not implement multipart push.
pub(super) async fn upload<B, V>(
    cx: &OpCx<'_, B>,
    request: MultipartUpdateRequest<'_, Pin<Box<dyn UploadReader>>, V>,
    uri: &str,
) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    V: Serialize + Send + Sync,
{
    cx.bmc()
        .multipart_update::<_, _, Value>(uri, request)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| match cx.map_bmc_error(error) {
            PlatformError::Bmc { status: 404, .. } => PlatformError::Unsupported,
            other => other,
        })
}
