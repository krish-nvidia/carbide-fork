/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::bios::attributes::lenovo::xcc as table;
use crate::bios::standard::{self, StandardBios};
use crate::resources::patch_bios_attributes;

/// Lenovo XCC names the UEFI administrator password `UefiAdminPassword`.
pub(crate) struct XccBios;

const UEFI_PASSWORD_NAME: &str = "UefiAdminPassword";

/// XCC names CPU virtualization after the CPU vendor.
const VIRTUALIZATION_ATTRIBUTES: [&str; 2] = [
    "Processors_IntelVirtualizationTechnology",
    "Processors_SVMMode",
];

/// Fails when the BIOS reports neither CPU vendor's virtualization attribute,
/// which leaves virtualization impossible to enable.
fn require_virtualization_attribute(current: &BiosSettings) -> Result<(), PlatformError> {
    if VIRTUALIZATION_ATTRIBUTES
        .iter()
        .any(|name| current.attributes.contains_key(*name))
    {
        return Ok(());
    }
    Err(PlatformError::InvalidResponse {
        message: format!(
            "BIOS reports neither {}",
            VIRTUALIZATION_ATTRIBUTES.join(" nor ")
        ),
    })
}

#[async_trait]
impl<B: Bmc> Bios<B> for XccBios
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
        let current = standard::current(cx).await?;
        require_virtualization_attribute(&current)?;
        standard::resolve(table::ATTRIBUTES, &current, overlay)
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

#[cfg(test)]
mod tests {
    use carbide_test_support::value_scenarios;
    use serde_json::json;

    use super::*;

    #[test]
    fn virtualization_needs_either_cpu_vendors_attribute() {
        value_scenarios!(run = |name: &str| require_virtualization_attribute(
            &serde_json::from_value(json!({"attributes": {name: "Disabled"}})).expect("settings")
        )
        .is_ok();
            "present" {
                "Processors_IntelVirtualizationTechnology" => true,
                "Processors_SVMMode" => true,
            }
            "missing" {
                "Processors_Other" => false,
            }
        );
    }
}
