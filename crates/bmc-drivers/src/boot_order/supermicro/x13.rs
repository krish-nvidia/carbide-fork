/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, ControllerAction, DriverOutcome, Fetched,
    OpCx, PlatformError,
};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ModificationResponse, ODataId};
use nv_redfish::resource::ResetType;
use nv_redfish::schema::computer_system::BootUpdate;
use serde::Deserialize;
use serde_json::json;

use crate::boot_order::standard::{configure, selector_matches, set_standard_override, status};
use crate::resources::{patch_bios_settings, selected_bios};

/// Supermicro X13 boot behavior.
///
/// The HTTP boot option only appears after `IPv4HTTPSupport` is enabled in
/// BIOS and the host reboots, so a missing option enables it and blocks on a
/// restart. Models without `Boot.BootOrder` expose the OEM `FixedBootOrder`
/// resource instead, which orders device classes and the UEFI network
/// adapters separately.
pub(crate) struct X13BootOrder;

const NETWORK: &str = "UEFI Network";
const HARD_DISK: &str = "UEFI Hard Disk";

/// The Supermicro `FixedBootOrder` OEM resource body.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FixedBootOrder {
    #[serde(default)]
    fixed_boot_order: Vec<String>,
    #[serde(rename = "UEFINetwork", default)]
    uefi_network: Vec<String>,
}

/// Whether the standard path failed because this model has no `Boot.BootOrder`:
/// either the property is absent or the BMC rejects writes to it.
fn boot_order_unavailable(error: &PlatformError) -> bool {
    matches!(error, PlatformError::Unsupported)
        || matches!(
            error,
            PlatformError::Bmc { status: 400, message_id, message }
                if message_id.as_deref().is_some_and(|id| id.ends_with("PropertyUnknown"))
                    && message.contains("BootOrder")
        )
}

async fn fixed_boot_order<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<(ODataId, FixedBootOrder), PlatformError> {
    let target = ODataId::from(format!(
        "{}/Oem/Supermicro/FixedBootOrder",
        cx.system()?.odata_id()
    ));
    let fetched = cx
        .bmc()
        .get::<Fetched<FixedBootOrder>>(&target)
        .await
        .map_err(|error| cx.map_bmc_error(error))?;
    Ok((target, FixedBootOrder::clone(&fetched)))
}

fn fixed_status(fbo: &FixedBootOrder, selector: &BootInterfaceSelector) -> BootOrderStatus {
    let network_first = fbo
        .fixed_boot_order
        .first()
        .is_some_and(|entry| entry.starts_with(NETWORK));
    let adapter_first = fbo
        .uefi_network
        .first()
        .is_some_and(|entry| selector_matches(entry, selector));
    BootOrderStatus {
        boot_interface_first: network_first && adapter_first,
        disk_enabled: fbo
            .fixed_boot_order
            .iter()
            .any(|entry| entry.starts_with(HARD_DISK)),
        // The fixed order has a single network slot, so no other network
        // device class can precede the selected adapter.
        other_network_options_disabled: true,
    }
}

async fn configure_fixed<B: Bmc>(
    cx: &OpCx<'_, B>,
    selector: &BootInterfaceSelector,
) -> Result<DriverOutcome, PlatformError> {
    let (target, mut fbo) = fixed_boot_order(cx).await?;
    if fixed_status(&fbo, selector).is_configured() {
        return Ok(DriverOutcome::complete());
    }
    let position = fbo
        .uefi_network
        .iter()
        .position(|entry| selector_matches(entry, selector))
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: "UEFI network adapter for selected interface".to_string(),
        })?;
    fbo.uefi_network.swap(0, position);

    // Device-class names carry device specifics, so keep whatever entry the
    // BMC reports for each class and fall back to the bare class name.
    let class_entry = |prefix: &str| {
        fbo.fixed_boot_order
            .iter()
            .find(|entry| entry.starts_with(prefix))
            .cloned()
            .unwrap_or_else(|| prefix.to_string())
    };
    let mut order = vec!["Disabled".to_string(); fbo.fixed_boot_order.len().max(2)];
    order[0] = class_entry(NETWORK);
    order[1] = class_entry(HARD_DISK);
    cx.patch_id(
        &target,
        None,
        &json!({"FixedBootOrder": order, "UEFINetwork": fbo.uefi_network}),
    )
    .await
    .map(DriverOutcome::from)
}

async fn enable_http_boot<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?;
    let attribute = bios
        .raw()
        .attributes
        .as_ref()
        .and_then(|attributes| {
            attributes
                .dynamic_properties
                .keys()
                .find(|key| key.starts_with("IPv4HTTPSupport"))
                .cloned()
        })
        .ok_or(PlatformError::Unsupported)?;
    match patch_bios_settings(cx, &json!({"Attributes": {attribute: "Enabled"}})).await? {
        task @ ModificationResponse::Task(_) => Ok(DriverOutcome::from(task)),
        ModificationResponse::Entity(_) | ModificationResponse::Empty => Ok(
            DriverOutcome::blocked(ControllerAction::Power(ResetType::GracefulRestart)),
        ),
    }
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for X13BootOrder {
    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        match status(cx, selector).await {
            Err(error) if boot_order_unavailable(&error) => {
                Ok(fixed_status(&fixed_boot_order(cx).await?.1, selector))
            }
            result => result,
        }
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        set_standard_override(cx, override_setting).await
    }

    async fn configure(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        match configure(cx, selector).await {
            Err(PlatformError::MissingBootOption { .. }) => enable_http_boot(cx).await,
            Err(error) if boot_order_unavailable(&error) => configure_fixed(cx, selector).await,
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fbo(order: &[&str], network: &[&str]) -> FixedBootOrder {
        FixedBootOrder {
            fixed_boot_order: order.iter().map(|entry| (*entry).to_string()).collect(),
            uefi_network: network.iter().map(|entry| (*entry).to_string()).collect(),
        }
    }

    #[test]
    fn fixed_boot_order_status_requires_network_class_and_adapter_first() {
        let selector = BootInterfaceSelector::Mac("b8:e9:24:17:6d:72".parse().unwrap());
        let configured = fbo(
            &[
                "UEFI Network:(B2/D0/F0) NVIDIA",
                "UEFI Hard Disk",
                "Disabled",
            ],
            &["UEFI HTTP IPv4 (MAC:B8E924176D72)", "UEFI PXE IPv4 other"],
        );
        assert!(fixed_status(&configured, &selector).is_configured());

        let wrong_adapter = fbo(
            &["UEFI Network", "UEFI Hard Disk"],
            &["UEFI PXE IPv4 other", "UEFI HTTP IPv4 (MAC:B8E924176D72)"],
        );
        assert!(!fixed_status(&wrong_adapter, &selector).boot_interface_first);
        assert!(
            !fixed_status(&fbo(&["UEFI Hard Disk", "UEFI Network"], &[]), &selector)
                .boot_interface_first
        );
    }

    #[test]
    fn only_property_unknown_boot_order_rejections_trigger_the_oem_fallback() {
        let rejected = PlatformError::Bmc {
            status: 400,
            message_id: Some("Base.1.4.PropertyUnknown".to_string()),
            message: "The property BootOrder is not in the list of valid properties".to_string(),
        };
        assert!(boot_order_unavailable(&rejected));
        assert!(boot_order_unavailable(&PlatformError::Unsupported));
        let other = PlatformError::Bmc {
            status: 400,
            message_id: Some("Base.1.4.PropertyValueNotInList".to_string()),
            message: "BootOrder".to_string(),
        };
        assert!(!boot_order_unavailable(&other));
    }
}
