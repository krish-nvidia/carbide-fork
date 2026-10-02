/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc, EntityTypeRef};

use serde_json::json;

use crate::bios::attributes::nvidia::viking as table;
use crate::bios::standard::{self, StandardBios};
use crate::resources::patch_bios_attributes;

/// NVIDIA DGX Viking: AMI firmware naming the password `AdminPassword`; BIOS
/// defaults are restored by clearing the host BIOS NVRAM through the
/// UpdateService OEM action.
pub(crate) struct VikingBios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

/// The firmware inventory entry whose NVRAM holds the host BIOS settings.
const HOST_BIOS_INVENTORY: &str = "HostBIOS_0";

/// Clears the host BIOS NVRAM through the NVIDIA UpdateService OEM action.
async fn clear_nvram<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    let service = cx
        .service_root()
        .update_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let host_bios = service
        .firmware_inventories()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .into_iter()
        .map(|inventory| inventory.raw())
        .find(|inventory| inventory.id == HOST_BIOS_INVENTORY)
        .ok_or(PlatformError::Unsupported)?
        .odata_id()
        .to_string();
    service
        .oem_nvidia_actions()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .clear_nvram(vec![host_bios])
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

#[async_trait]
impl<B: Bmc> Bios<B> for VikingBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn expected(
        &self,
        cx: &OpCx<'_, B>,
        overlay: &BiosSettings,
    ) -> Result<BiosSettings, PlatformError> {
        standard::expected(cx, table::ATTRIBUTES, overlay).await
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        clear_nvram(cx).await
    }

    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        patch_bios_attributes(
            cx,
            json!({"TpmOperation": "TPM Clear", "TpmSupport": "Enable"}),
        )
        .await
    }

    /// Viking firmware does not support reverting staged settings.
    async fn clear_pending(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
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
