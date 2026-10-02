/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use crate::bios::attributes::BiosAttribute;
use crate::bios::attributes::nvidia::{gb200, vera_rubin};
use crate::bios::standard::{self, StandardBios};

/// NVIDIA OpenBMC platforms (GB200/GB300, Vera Rubin, GH) name the UEFI
/// administrator password `AdminPassword`; the model decides the attributes
/// machine setup expects.
pub(crate) struct OpenBmcBios {
    attributes: &'static [BiosAttribute],
}

impl OpenBmcBios {
    /// GH200, whose setup expects nothing beyond the caller's profile.
    pub(crate) const GH200: Self = Self { attributes: &[] };
    /// GB200 and GB300 NVL compute trays.
    pub(crate) const GBX00: Self = Self {
        attributes: gb200::ATTRIBUTES,
    };
    /// Vera Rubin NVL compute trays.
    pub(crate) const VERA_RUBIN: Self = Self {
        attributes: vera_rubin::ATTRIBUTES,
    };
}

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

#[async_trait]
impl<B: Bmc> Bios<B> for OpenBmcBios
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
        standard::expected(cx, self.attributes, overlay).await
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
