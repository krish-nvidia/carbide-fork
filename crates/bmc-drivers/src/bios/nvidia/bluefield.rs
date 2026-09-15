/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use serde_json::json;

use crate::bios::standard::StandardBios;
use crate::resources::patch_bios_attributes;

/// NVIDIA BlueField: the DPU BIOS exposes no `ResetBios` or `ChangePassword`
/// actions; both are write-only attributes on the pending settings.
pub(crate) struct BlueFieldBios;

async fn set_password_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError> {
    patch_bios_attributes(
        cx,
        json!({
            "CurrentUefiPassword": current_password,
            "UefiPassword": new_password,
        }),
    )
    .await
}

#[async_trait]
impl<B: Bmc> Bios<B> for BlueFieldBios {
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        patch_bios_attributes(cx, json!({"ResetEfiVars": true})).await
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        set_password_attributes(cx, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        set_password_attributes(cx, current_password, "").await
    }
}
