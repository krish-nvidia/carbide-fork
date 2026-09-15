/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::bios::standard::{self, StandardBios};

/// AMI MegaRAC (including Lenovo AMI-based systems) names the UEFI
/// administrator password `SETUP001`.
pub(crate) struct MegaRacBios;

const UEFI_PASSWORD_NAME: &str = "SETUP001";

#[async_trait]
impl<B: Bmc> Bios<B> for MegaRacBios {
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
}
