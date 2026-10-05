/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use crate::bios::attributes::BiosAttribute;
use crate::bios::standard::StandardBios;
use crate::bios::support::{
    attribute_holds, change_password, compare, current_settings, expected, stage,
};

/// NVIDIA GB200 and GB300 NVL compute trays: OpenBMC naming the UEFI
/// administrator password `AdminPassword`.
pub(crate) struct Gbx00Bios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

/// Option ROMs stay enabled so the DPU appears among the host's network
/// devices and boot options.
const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("TPM", "Enabled").required(),
    INFINITE_BOOT,
    BiosAttribute::bool("Socket0Pcie6DisableOptionROM", false),
    BiosAttribute::bool("Socket1Pcie6DisableOptionROM", false),
];

/// Disabling the embedded UEFI shell keeps the BIOS retrying boot instead of
/// dropping into the shell.
const INFINITE_BOOT: BiosAttribute =
    BiosAttribute::string("EmbeddedUefiShell", "Disabled").required();

#[async_trait]
impl<B: Bmc> Bios<B> for Gbx00Bios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = current_settings(cx).await?;
        stage(cx, &expected(ATTRIBUTES, &current, profile)).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let current = current_settings(cx).await?;
        Ok(compare(&current, &expected(ATTRIBUTES, &current, profile)))
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await
    }

    async fn infinite_boot_enabled(&self, cx: &OpCx<'_, B>) -> Result<Option<bool>, PlatformError> {
        attribute_holds(cx, INFINITE_BOOT).await
    }
}
