/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{Bmc, EntityTypeRef, ODataId};
use serde_json::json;

use crate::bios::standard::{self, StandardBios};

/// NVIDIA DGX Viking: AMI firmware naming the password `AdminPassword`; BIOS
/// defaults are restored by clearing the host BIOS NVRAM through the
/// UpdateService OEM action.
pub(crate) struct VikingBios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

/// Clears the host BIOS NVRAM through the NVIDIA UpdateService OEM action.
async fn clear_nvram<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let service = cx
        .service_root()
        .update_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let raw = service.raw();
    let target = ODataId::from(format!(
        "{}/Actions/Oem/NvidiaUpdateService.ClearNVRAM",
        raw.odata_id()
    ));
    let host_bios = format!("{}/FirmwareInventory/HostBIOS_0", raw.odata_id());
    cx.post(&target, &json!({"Targets": [host_bios]})).await
}

#[async_trait]
impl<B: Bmc> Bios<B> for VikingBios {
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        clear_nvram(cx).await
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::change_password(cx, UEFI_PASSWORD_NAME, current_password, "").await
    }
}
