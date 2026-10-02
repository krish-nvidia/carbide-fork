/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use serde_json::json;

use crate::bios::attributes::hpe::ilo as table;
use crate::bios::standard::{self, StandardBios};
use crate::resources::patch_bios_attributes;

/// HPE iLO BIOS behavior: staged settings are left in place rather than
/// reverted, and the TPM is cleared through BIOS attributes.
pub(crate) struct IloBios;

#[async_trait]
impl<B: Bmc> Bios<B> for IloBios
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

    async fn clear_pending(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }

    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        patch_bios_attributes(
            cx,
            json!({"Tpm2Operation": "Clear", "TpmVisibility": "Visible"}),
        )
        .await
    }
}
