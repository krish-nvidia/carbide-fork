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
use crate::bios::support::{change_password, compare, current_settings, expected, stage};

/// Supermicro GB300 NVL compute trays: NVIDIA tray firmware naming the UEFI
/// administrator password `AdminPassword`, with the TPM behind an AMI BIOS
/// attribute.
pub(crate) struct Gb300Bios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

/// Option ROMs stay enabled so the DPU appears among the host's network
/// devices and boot options.
const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("SecurityDeviceSupport", "Enabled").required(),
    BiosAttribute::bool("Socket0Pcie6DisableOptionROM", false),
    BiosAttribute::bool("Socket1Pcie6DisableOptionROM", false),
];

#[async_trait]
impl<B: Bmc> Bios<B> for Gb300Bios
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
        let current = current_settings(cx).await?;
        stage(cx, &expected(ATTRIBUTES, &current, profile)).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
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
}
