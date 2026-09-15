/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Firmware, OpCx, PlatformError};
use nv_redfish::core::{Bmc, MultipartUpdateRequest};
use nv_redfish::update_service::MultipartUpdateParameters;
use serde_json::Value;

use crate::firmware::standard::{
    StandardFirmware, UploadRequest, advertised_multipart_uri, update_service, upload,
};

/// NVIDIA OpenBMC trays: standard multipart upload with `ForceUpdate`, since the
/// BMC otherwise skips images matching the installed version.
pub(crate) struct OpenBmcFirmware;

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

#[async_trait]
impl<B: Bmc> Firmware<B> for OpenBmcFirmware {
    fn standard(&self) -> &dyn Firmware<B> {
        &StandardFirmware
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        request: UploadRequest<'_>,
    ) -> Result<DriverOutcome, PlatformError> {
        let service = update_service(cx).await?;
        let uri = advertised_multipart_uri(&service).ok_or(PlatformError::Unsupported)?;
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
}
