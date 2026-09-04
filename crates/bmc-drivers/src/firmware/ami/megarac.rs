/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Firmware, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::schema::software_inventory::SoftwareInventory;
use nv_redfish::schema::update_service::UpdateServiceSimpleUpdateAction;
use nv_redfish::update_service::{MultipartUpdateParameters, UpdateService};
use serde_json::json;

use crate::firmware::standard::{self, StandardFirmware, UploadRequest};

/// AMI MegaRAC firmware behavior for Lenovo HS350x-class BMCs.
///
/// The BMC accepts only its BIOS or BMC image targets with `OemParameters`,
/// and a BMC image wipes configuration unless preservation is requested first.
pub(crate) struct MegaRacFirmware {
    /// Standard upload, with MegaRAC's `upload` path when none is advertised.
    upload: StandardFirmware,
}

pub(crate) static MEGARAC_FIRMWARE: MegaRacFirmware = MegaRacFirmware {
    upload: StandardFirmware {
        multipart_fallback: Some("/redfish/v1/UpdateService/upload"),
        force_update: false,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Image {
    Bmc,
    Bios,
}

fn image(parameters: &MultipartUpdateParameters) -> Result<Image, PlatformError> {
    match parameters
        .targets
        .as_deref()
        .ok_or(PlatformError::Unsupported)?
    {
        [target] if target.ends_with("/BIOSImage1") => Ok(Image::Bios),
        [first, second] if first.ends_with("/BMCImage1") && second.ends_with("/BMCImage2") => {
            Ok(Image::Bmc)
        }
        _ => Err(PlatformError::Unsupported),
    }
}

async fn preserve_bmc_configuration<B: Bmc>(
    cx: &OpCx<'_, B>,
    service: &UpdateService<B>,
) -> Result<(), PlatformError> {
    let preserve = json!({"Oem": {"AMIUpdateService": {"PreserveConfiguration": {
        "Authentication": true, "EXTLOG": true, "FRU": true, "IPMI": true,
        "KVM": true, "NTP": true, "Network": true, "REDFISH": true,
        "SDR": true, "SEL": true, "SNMP": true, "SSH": true,
        "Syslog": true, "WEB": true
    }}}});
    match cx.patch(service.raw().as_ref(), &preserve).await? {
        DriverOutcome::Complete { .. } => Ok(()),
        DriverOutcome::Accepted { .. } | DriverOutcome::Blocked { .. } => {
            Err(PlatformError::Unsupported)
        }
    }
}

#[async_trait]
impl<B> Firmware<B> for MegaRacFirmware
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn inventory(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<Vec<Arc<SoftwareInventory>>, PlatformError> {
        standard::inventory(cx).await
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        request: UploadRequest<'_>,
    ) -> Result<DriverOutcome, PlatformError> {
        let image = image(request.update_parameters)?;
        if !request
            .oem_parts
            .iter()
            .any(|part| part.name == "OemParameters")
        {
            return Err(PlatformError::Unsupported);
        }
        if image == Image::Bmc {
            let service = standard::update_service(cx).await?;
            preserve_bmc_configuration(cx, &service).await?;
        }
        let uri = self.upload.upload_uri(cx).await?;
        standard::upload(cx, request, &uri).await
    }

    async fn simple_update(
        &self,
        cx: &OpCx<'_, B>,
        request: &UpdateServiceSimpleUpdateAction,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::simple_update(cx, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bios_and_paired_bmc_image_targets_are_accepted() {
        let targets = |targets: &[&str]| {
            MultipartUpdateParameters::builder()
                .with_targets(targets.iter().map(|target| (*target).to_string()).collect())
                .build()
        };
        assert_eq!(
            image(&targets(&[
                "/redfish/v1/UpdateService/FirmwareInventory/BIOSImage1"
            ])),
            Ok(Image::Bios)
        );
        assert_eq!(
            image(&targets(&[
                "/redfish/v1/UpdateService/FirmwareInventory/BMCImage1",
                "/redfish/v1/UpdateService/FirmwareInventory/BMCImage2",
            ])),
            Ok(Image::Bmc)
        );
        assert_eq!(
            image(&targets(&[
                "/redfish/v1/UpdateService/FirmwareInventory/PSU1"
            ])),
            Err(PlatformError::Unsupported)
        );
    }
}
