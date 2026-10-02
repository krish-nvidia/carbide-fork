/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Firmware, FirmwareComponent, FirmwareUpload, OpCx, PlatformError,
};
use nv_redfish::core::{Bmc, OemMultipartPart};
use nv_redfish::oem::ami::update_service::{
    AmiUpdateServiceUpdate, AmiUpdateServiceUpdateExt, ImageType, OemParametersUpdate,
    PreserveConfigurationUpdate, oem_parameters_part,
};
use nv_redfish::update_service::{MultipartUpdateParameters, UpdateService, UpdateServiceUpdate};

use crate::firmware::standard::{StandardFirmware, multipart_upload, update_service};
use crate::firmware::support::{inventory_targets, upload_uri};

/// AMI MegaRAC firmware behavior for Lenovo HS350x-class BMCs.
///
/// The BMC accepts only its BIOS or BMC images, targeting their firmware
/// inventory entries with `OemParameters` naming the image, uploads through
/// `upload` when `MultipartUpload` is not advertised, and a BMC image wipes
/// configuration unless preservation is requested first.
pub(crate) struct MegaRacFirmware;

const MULTIPART_UPLOAD: &str = "/redfish/v1/UpdateService/upload";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Image {
    Bmc,
    Bios,
}

/// The image `component` names and the firmware inventory entries it targets.
fn image(component: FirmwareComponent) -> Result<(Image, &'static [&'static str]), PlatformError> {
    match component {
        FirmwareComponent::Bmc => Ok((Image::Bmc, &["BMCImage1", "BMCImage2"])),
        FirmwareComponent::Uefi => Ok((Image::Bios, &["BIOSImage1"])),
        _ => Err(PlatformError::Unsupported),
    }
}

/// The `OemParameters` part naming `image`; a BIOS image keeps its NVRAM.
fn oem_parameters(image: Image) -> Result<OemMultipartPart, PlatformError> {
    let parameters = match image {
        Image::Bmc => OemParametersUpdate::builder()
            .with_image_type(ImageType::Bmc)
            .build(),
        Image::Bios => OemParametersUpdate::builder()
            .with_image_type(ImageType::Bios)
            .with_preserve_bios("true".to_string())
            .build(),
    };
    oem_parameters_part(&parameters).map_err(|error| PlatformError::InvalidResponse {
        message: format!("failed to serialize AMI OemParameters: {error}"),
    })
}

async fn preserve_bmc_configuration<B: Bmc>(
    cx: &OpCx<'_, B>,
    service: &UpdateService<B>,
) -> Result<(), PlatformError> {
    let preserve = PreserveConfigurationUpdate::builder()
        .with_authentication(true)
        .with_extlog(true)
        .with_fru(true)
        .with_ipmi(true)
        .with_kvm(true)
        .with_ntp(true)
        .with_network(true)
        .with_redfish(true)
        .with_sdr(true)
        .with_sel(true)
        .with_snmp(true)
        .with_ssh(true)
        .with_syslog(true)
        .with_web(true)
        .build();
    let body = UpdateServiceUpdate::builder()
        .build()
        .with_oem_ami(
            AmiUpdateServiceUpdate::builder()
                .with_preserve_configuration(preserve)
                .build(),
        )
        .map_err(|error| PlatformError::InvalidResponse {
            message: format!("failed to build the AMI preserve-configuration update: {error}"),
        })?;
    match service
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))?
    {
        DriverOutcome::Complete { .. } => Ok(()),
        DriverOutcome::Accepted { .. } | DriverOutcome::Blocked { .. } => {
            Err(PlatformError::Unsupported)
        }
    }
}

#[async_trait]
impl<B: Bmc> Firmware<B> for MegaRacFirmware {
    fn standard(&self) -> &dyn Firmware<B> {
        &StandardFirmware
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        upload: FirmwareUpload,
    ) -> Result<DriverOutcome, PlatformError> {
        let (image, inventory) = image(upload.component)?;
        let parameters = MultipartUpdateParameters::builder()
            .with_targets(inventory_targets(cx, inventory).await?)
            .build();
        if image == Image::Bmc {
            let service = update_service(cx).await?;
            preserve_bmc_configuration(cx, &service).await?;
        }
        let uri = upload_uri(cx, MULTIPART_UPLOAD).await?;
        multipart_upload(cx, upload, &parameters, vec![oem_parameters(image)?], &uri).await
    }
}
