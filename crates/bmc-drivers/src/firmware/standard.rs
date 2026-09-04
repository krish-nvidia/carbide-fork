/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Firmware, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc, MultipartUpdateRequest, UploadReader};
use nv_redfish::schema::software_inventory::SoftwareInventory;
use nv_redfish::schema::update_service::UpdateServiceSimpleUpdateAction;
use nv_redfish::update_service::{MultipartUpdateParameters, UpdateService};
use serde::Serialize;
use serde_json::Value;

pub(super) type UploadRequest<'a> =
    MultipartUpdateRequest<'a, Pin<Box<dyn UploadReader>>, MultipartUpdateParameters>;

/// Standard Redfish inventory, SimpleUpdate, and multipart upload.
pub(crate) struct StandardFirmware {
    /// Upload endpoint for firmware that does not advertise `MultipartHttpPushUri`.
    pub(crate) multipart_fallback: Option<&'static str>,
    /// Ask the BMC to flash even when the image version matches what is installed.
    pub(crate) force_update: bool,
}

pub(crate) static STANDARD_FIRMWARE: StandardFirmware = StandardFirmware {
    multipart_fallback: None,
    force_update: false,
};

#[async_trait]
impl<B> Firmware<B> for StandardFirmware
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn inventory(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<Arc<SoftwareInventory>>, PlatformError> {
        inventory(cx).await
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        request: UploadRequest<'_>,
    ) -> Result<DriverOutcome, PlatformError> {
        let uri = self.upload_uri(cx).await?;
        if !self.force_update {
            return upload(cx, request, &uri).await;
        }
        let parameters = forced(request.update_parameters)?;
        upload(
            cx,
            MultipartUpdateRequest {
                update_parameters: &parameters,
                update_stream: request.update_stream,
                oem_parts: request.oem_parts,
                upload_timeout: request.upload_timeout,
            },
            &uri,
        )
        .await
    }

    async fn simple_update(
        &self,
        cx: &OpCx<'_, B>,
        request: &UpdateServiceSimpleUpdateAction,
    ) -> Result<DriverOutcome, PlatformError> {
        simple_update(cx, request).await
    }
}

impl StandardFirmware {
    /// The advertised `MultipartHttpPushUri`, or this family's known upload path.
    pub(super) async fn upload_uri<B: Bmc>(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<String, PlatformError> {
        let service = update_service(cx).await?;
        advertised_multipart_uri(&service)
            .or_else(|| self.multipart_fallback.map(str::to_string))
            .ok_or(PlatformError::Unsupported)
    }
}

/// The generated parameters type is not `Clone`, so `ForceUpdate` is set on
/// its JSON form.
fn forced(parameters: &MultipartUpdateParameters) -> Result<Value, PlatformError> {
    let mut parameters =
        serde_json::to_value(parameters).map_err(|error| PlatformError::InvalidResponse {
            message: format!("failed to serialize update parameters: {error}"),
        })?;
    parameters["ForceUpdate"] = Value::Bool(true);
    Ok(parameters)
}

pub(super) async fn update_service<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<UpdateService<B>, PlatformError> {
    cx.service_root()
        .update_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

pub(super) async fn inventory<B: Bmc>(
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

pub(super) async fn simple_update<B>(
    cx: &OpCx<'_, B>,
    request: &UpdateServiceSimpleUpdateAction,
) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    B::Error: ActionError,
{
    update_service(cx)
        .await?
        .raw()
        .actions
        .as_ref()
        .ok_or(PlatformError::Unsupported)?
        .simple_update(cx.bmc(), request)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_bmc_error(error))
}
