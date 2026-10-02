/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish firmware operations.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Firmware, FirmwareUpload, OpCx, OperationReference, PlatformError,
};
use nv_redfish::core::{
    Bmc, ModificationResponse, MultipartUpdateRequest, ODataId, OemMultipartPart, UploadReader,
};
use nv_redfish::schema::message::Message;
use nv_redfish::schema::software_inventory::SoftwareInventory;
use nv_redfish::schema::update_service::UpdateServiceSimpleUpdateAction;
use nv_redfish::update_service::{MultipartUpdateParameters, UpdateService};
use serde::{Deserialize, Serialize};

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

    /// Uploads through the advertised `MultipartHttpPushUri` with empty
    /// parameters, leaving the target to the image.
    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        upload: FirmwareUpload,
    ) -> Result<DriverOutcome, PlatformError> {
        let service = update_service(cx).await?;
        let uri = advertised_multipart_uri(&service).ok_or(PlatformError::Unsupported)?;
        multipart_upload(
            cx,
            upload,
            &MultipartUpdateParameters::default(),
            Vec::new(),
            &uri,
        )
        .await
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

/// The message announcing the task an upload started.
const TASK_STARTED: &str = "Task.1.0.New";

/// An upload answered without `202 Accepted`: the started task is announced
/// in `Messages`, or the body is the task itself.
#[derive(Deserialize)]
pub(super) struct UploadResponse {
    #[serde(rename = "@odata.id")]
    odata_id: Option<ODataId>,
    #[serde(rename = "@odata.type")]
    odata_type: Option<String>,
    #[serde(rename = "Messages", default)]
    messages: Vec<Message>,
}

/// The task an upload started, or completion when it started none.
pub(super) fn upload_outcome(response: ModificationResponse<UploadResponse>) -> DriverOutcome {
    let ModificationResponse::Entity(body) = response else {
        return DriverOutcome::from(response);
    };
    let announced = body.messages.iter().find_map(|message| {
        (message.message_id == TASK_STARTED)
            .then(|| message.message_args.as_ref()?.first().cloned())
            .flatten()
    });
    let task = body
        .odata_type
        .as_deref()
        .is_some_and(|odata_type| odata_type.starts_with("#Task."));
    match announced
        .map(ODataId::from)
        .or(body.odata_id.filter(|_| task))
    {
        Some(uri) => DriverOutcome::accepted(OperationReference::RedfishTask {
            uri,
            retry_after_seconds: None,
        }),
        None => DriverOutcome::complete(),
    }
}

/// Uploads `upload` to `uri` with `parameters` and any OEM parts; a 404 means
/// the BMC does not implement multipart push.
pub(super) async fn multipart_upload<B, V>(
    cx: &OpCx<'_, B>,
    upload: FirmwareUpload,
    parameters: &V,
    oem_parts: Vec<OemMultipartPart>,
    uri: &str,
) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    V: Serialize + Send + Sync,
{
    let request: MultipartUpdateRequest<'_, Pin<Box<dyn UploadReader>>, V> =
        MultipartUpdateRequest {
            update_parameters: parameters,
            update_stream: upload.image,
            oem_parts,
            upload_timeout: upload.timeout,
        };
    cx.bmc()
        .multipart_update::<_, _, UploadResponse>(uri, request)
        .await
        .map(upload_outcome)
        .map_err(|error| match cx.map_bmc_error(error) {
            PlatformError::Bmc { status: 404, .. } => PlatformError::Unsupported,
            other => other,
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_upload_answered_with_a_task_message_is_tracked_as_that_task() {
        let entity = |body| {
            ModificationResponse::Entity(
                serde_json::from_value::<UploadResponse>(body).expect("upload response"),
            )
        };
        assert_eq!(
            upload_outcome(entity(json!({"Messages": [{
                "MessageId": TASK_STARTED,
                "MessageArgs": ["/redfish/v1/TaskService/Tasks/3"],
            }]}))),
            DriverOutcome::accepted(OperationReference::RedfishTask {
                uri: "/redfish/v1/TaskService/Tasks/3".to_string().into(),
                retry_after_seconds: None,
            })
        );
        assert_eq!(
            upload_outcome(entity(json!({"Messages": []}))),
            DriverOutcome::complete()
        );
    }
}
