/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::bios::standard::{self, StandardBios};
use crate::dell;

/// Dell iDRAC BIOS behavior.
///
/// iDRAC applies staged settings only through a configuration job, refuses
/// new jobs while any is queued, and clears pending settings by deleting the
/// job queue rather than by re-writing attributes.
pub(crate) struct IdracBios;

/// iDRAC exposes the UEFI administrator password as `SetupPassword`.
const UEFI_PASSWORD_NAME: &str = "SetupPassword";

async fn change_password<B: Bmc>(
    cx: &OpCx<'_, B>,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError> {
    dell::clear_job_queue(cx).await?;
    standard::change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await?;
    dell::create_bios_config_job(cx).await
}

#[async_trait]
impl<B: Bmc> Bios<B> for IdracBios {
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        if self.standard().status(cx, expected).await?.is_applied {
            return Ok(DriverOutcome::complete());
        }
        let attributes = serde_json::to_value(&expected.attributes).map_err(|error| {
            PlatformError::InvalidResponse {
                message: format!("failed to serialize BIOS attributes: {error}"),
            }
        })?;
        dell::stage_bios_attributes(cx, attributes).await
    }

    async fn clear_pending(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        dell::clear_job_queue(cx).await?;
        Ok(DriverOutcome::complete())
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password(cx, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password(cx, current_password, "").await
    }
}
