/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Bios, BiosDiff, BiosSettings, BiosStatus, BootInterfaceSelector, DriverOutcome, OpCx,
    PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::{Value, json};

use crate::bios::attributes::BiosAttribute;
use crate::bios::standard::StandardBios;
use crate::bios::support::{
    attribute_holds, change_password, compare, current_settings, expected, settings, stage,
};
use crate::boot_order::lenovo::{NETWORK, first_group, network_group_first};

/// Lenovo XCC names the UEFI administrator password `UefiAdminPassword`.
pub(crate) struct XccBios;

const UEFI_PASSWORD_NAME: &str = "UefiAdminPassword";

/// Virtualization is checked by [`require_virtualization_attribute`] since
/// each CPU vendor reports only its own attribute.
const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("DevicesandIOPorts_COMPort1", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_ConsoleRedirection", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_SerialPortSharing", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_SPRedirection", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_COMPortActiveAfterBoot", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_SerialPortAccessMode", "Shared"),
    BiosAttribute::string("Processors_IntelVirtualizationTechnology", "Enabled"),
    BiosAttribute::string("Processors_SVMMode", "Enabled"),
    BiosAttribute::string("BootModes_SystemBootMode", "UEFIMode").required(),
    BiosAttribute::string("NetworkStackSettings_IPv4HTTPSupport", "Enabled").required(),
    BiosAttribute::string("NetworkStackSettings_IPv4PXESupport", "Disabled").required(),
    BiosAttribute::string("NetworkStackSettings_IPv6PXESupport", "Disabled").required(),
    INFINITE_BOOT,
    BiosAttribute::string("BootModes_PreventOSChangesToBootOrder", "Enabled").required(),
    // Only older systems still have a legacy BIOS mode.
    BiosAttribute::string("LegacyBIOS_NonOnboardPXE", "Disabled"),
    BiosAttribute::string("LegacyBIOS_LegacyBIOS", "Disabled"),
];

const INFINITE_BOOT: BiosAttribute =
    BiosAttribute::string("BootModes_InfiniteBootRetry", "Enabled").required();

fn tpm_clear() -> BiosSettings {
    settings([("TrustedComputingGroup_DeviceOperation", json!("Clear"))])
}

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

fn expected_settings(
    current: &BiosSettings,
    profile: &BiosSettings,
) -> Result<BiosSettings, PlatformError> {
    require_virtualization_attribute(current)?;
    Ok(expected(ATTRIBUTES, current, profile))
}

/// The status difference reporting which device group boots first.
const FIRST_BOOT_GROUP: &str = "boot_first_type";

#[async_trait]
impl<B: Bmc> Bios<B> for XccBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    /// Also puts the network device group first so boot order setup, after
    /// the reset, finds the adapters it orders.
    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = current_settings(cx).await?;
        let settings = stage(cx, &expected_settings(&current, profile)?).await?;
        Ok(settings.merge(network_group_first(cx).await?))
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<BiosStatus, PlatformError> {
        let current = current_settings(cx).await?;
        let mut status = compare(&current, &expected_settings(&current, profile)?);
        let group = first_group(cx).await?;
        if group.as_deref() != Some(NETWORK) {
            status.differences.push(BiosDiff {
                key: FIRST_BOOT_GROUP.to_string(),
                expected: json!(NETWORK),
                actual: group.map(Value::from),
            });
            status.is_applied = false;
        }
        Ok(status)
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await
    }

    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        stage(cx, &tpm_clear()).await
    }

    async fn infinite_boot_enabled(&self, cx: &OpCx<'_, B>) -> Result<Option<bool>, PlatformError> {
        attribute_holds(cx, INFINITE_BOOT).await
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
