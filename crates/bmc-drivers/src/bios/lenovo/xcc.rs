/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use serde_json::json;

use crate::bios::standard::{self, StandardBios};
use crate::resources::patch_bios_attributes;

/// Lenovo XCC names the UEFI administrator password `UefiAdminPassword`.
pub(crate) struct XccBios;

const UEFI_PASSWORD_NAME: &str = "UefiAdminPassword";

#[async_trait]
impl<B: Bmc> Bios<B> for XccBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
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

    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        patch_bios_attributes(
            cx,
            json!({"TrustedComputingGroup_DeviceOperation": "Clear"}),
        )
        .await
    }
}
