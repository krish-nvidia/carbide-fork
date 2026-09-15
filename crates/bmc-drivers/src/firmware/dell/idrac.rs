/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Firmware, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::firmware::standard::{StandardFirmware, UploadRequest, upload};
use crate::firmware::support::upload_uri;

/// Dell iDRAC: older iDRAC firmware does not advertise its `MultipartUpload` endpoint.
pub(crate) struct IdracFirmware;

const MULTIPART_UPLOAD: &str = "/redfish/v1/UpdateService/MultipartUpload";

#[async_trait]
impl<B: Bmc> Firmware<B> for IdracFirmware {
    fn standard(&self) -> &dyn Firmware<B> {
        &StandardFirmware
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        request: UploadRequest<'_>,
    ) -> Result<DriverOutcome, PlatformError> {
        let uri = upload_uri(cx, MULTIPART_UPLOAD).await?;
        upload(cx, request, &uri).await
    }
}
