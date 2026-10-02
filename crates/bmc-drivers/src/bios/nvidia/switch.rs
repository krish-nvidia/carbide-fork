/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use crate::bios::standard::{self, StandardBios};

/// NVIDIA GB NVSwitch trays expose no BIOS attributes, but their BIOS still
/// accepts `Bios.ChangePassword` for the `AdminPassword` slot.
pub(crate) struct SwitchBios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

#[async_trait]
impl<B: Bmc> Bios<B> for SwitchBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn current(&self, _cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn pending(&self, _cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn status(
        &self,
        _cx: &OpCx<'_, B>,
        _expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn apply(
        &self,
        _cx: &OpCx<'_, B>,
        _expected: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn reset(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn clear_pending(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
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
