/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Bios, BiosSettings, BiosStatus, BootInterfaceSelector, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};

use crate::bios::attributes::BiosAttribute;
use crate::bios::standard::StandardBios;
use crate::bios::support::{RedfishBiosExt as _, compare, expected};
use crate::resources::RedfishResourcesExt as _;

/// NVIDIA Vera Rubin NVL compute trays: OpenBMC naming the UEFI administrator
/// password `AdminPassword`, with GPUs exposed as PCIe devices.
pub(crate) struct VeraRubinBios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("TPM", "Enabled").required(),
    INFINITE_BOOT,
    BiosAttribute::bool("GpuExposeAsPcie", true).required(),
];

/// Disabling the embedded UEFI shell keeps the BIOS retrying boot instead of
/// dropping into the shell.
const INFINITE_BOOT: BiosAttribute =
    BiosAttribute::string("EmbeddedUefiShell", "Disabled").required();

#[async_trait]
impl<B: Bmc> Bios<B> for VeraRubinBios
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
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = cx.current_bios_settings().await?;
        cx.stage_bios_attributes(&expected(ATTRIBUTES, &current, profile).attributes)
            .await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<BiosStatus, PlatformError> {
        let current = cx.current_bios_settings().await?;
        Ok(compare(&current, &expected(ATTRIBUTES, &current, profile)))
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        cx.change_bios_password(UEFI_PASSWORD_NAME, current_password, new_password)
            .await
    }

    async fn infinite_boot_enabled(&self, cx: &OpCx<'_, B>) -> Result<Option<bool>, PlatformError> {
        cx.bios_attribute_holds(INFINITE_BOOT).await
    }
}
