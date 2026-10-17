/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Firmware, FirmwareUpload, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::schema::resource::OemUpdate;
use nv_redfish::update_service::{
    MultipartUpdateParameters, MultipartUpdateParametersWithApplyTime, OperationApplyTime,
};
use serde_json::json;

use crate::firmware::standard::StandardFirmware;
use crate::firmware::support::RedfishFirmwareExt as _;

/// Dell iDRAC: older iDRAC firmware does not advertise its `MultipartUpload`
/// endpoint; the image picks its own target and applies at once or on the
/// next reset as the caller asks.
pub(crate) struct IdracFirmware;

const MULTIPART_UPLOAD: &str = "/redfish/v1/UpdateService/MultipartUpload";

fn update_parameters(apply_immediately: bool) -> MultipartUpdateParametersWithApplyTime {
    MultipartUpdateParametersWithApplyTime {
        parameters: MultipartUpdateParameters::builder()
            .with_targets(Vec::new())
            .with_oem(OemUpdate {
                additional_properties: json!({}),
            })
            .build(),
        operation_apply_time: Some(if apply_immediately {
            OperationApplyTime::Immediate
        } else {
            OperationApplyTime::OnReset
        }),
    }
}

#[async_trait]
impl<B: Bmc> Firmware<B> for IdracFirmware {
    fn standard(&self) -> &dyn Firmware<B> {
        &StandardFirmware
    }

    async fn multipart_update(
        &self,
        cx: &OpCx<'_, B>,
        upload: FirmwareUpload,
    ) -> Result<DriverOutcome, PlatformError> {
        let parameters = update_parameters(upload.apply_immediately);
        let uri = cx.firmware_upload_uri(MULTIPART_UPLOAD).await?;
        cx.multipart_upload(upload, &parameters, Vec::new(), &uri)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_parameters_name_no_targets_and_the_requested_apply_time() {
        for (apply_immediately, apply_time) in [(true, "Immediate"), (false, "OnReset")] {
            assert_eq!(
                serde_json::to_value(update_parameters(apply_immediately)).expect("serializes"),
                json!({"Targets": [], "Oem": {}, "@Redfish.OperationApplyTime": apply_time})
            );
        }
    }
}
