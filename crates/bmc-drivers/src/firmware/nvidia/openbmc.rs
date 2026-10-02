/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Firmware, FirmwareComponent, FirmwareUpload, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;
use nv_redfish::update_service::MultipartUpdateParameters;

use crate::firmware::standard::{
    StandardFirmware, advertised_multipart_uri, multipart_upload, update_service,
};
use crate::firmware::support::{chassis_target, inventory_targets};

/// NVIDIA OpenBMC compute trays (GB200/GB300, Vera Rubin): the HGX component
/// is the update target, and `ForceUpdate` keeps the BMC from skipping images
/// that match the installed version.
pub(crate) struct OpenBmcFirmware;

/// The update targets for `component`; `None` leaves the target to the image.
async fn targets<B: Bmc>(
    cx: &OpCx<'_, B>,
    component: FirmwareComponent,
) -> Result<Option<Vec<String>>, PlatformError> {
    Ok(match component {
        FirmwareComponent::Unknown => None,
        FirmwareComponent::Bmc => Some(Vec::new()),
        FirmwareComponent::ErotBmc => Some(vec![chassis_target(cx, "HGX_ERoT_BMC_0").await?]),
        FirmwareComponent::ErotBios => Some(inventory_targets(cx, &["EROT_BIOS_0"]).await?),
        FirmwareComponent::HgxBmc | FirmwareComponent::Uefi => {
            Some(vec![chassis_target(cx, "HGX_Chassis_0").await?])
        }
        _ => return Err(PlatformError::Unsupported),
    })
}

#[async_trait]
impl<B: Bmc> Firmware<B> for OpenBmcFirmware {
    fn standard(&self) -> &dyn Firmware<B> {
        &StandardFirmware
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        upload: FirmwareUpload,
    ) -> Result<DriverOutcome, PlatformError> {
        let service = update_service(cx).await?;
        let uri = advertised_multipart_uri(&service).ok_or(PlatformError::Unsupported)?;
        let mut parameters = MultipartUpdateParameters::builder().with_force_update(true);
        if let Some(targets) = targets(cx, upload.component).await? {
            parameters = parameters.with_targets(targets);
        }
        multipart_upload(cx, upload, &parameters.build(), Vec::new(), &uri).await
    }
}
