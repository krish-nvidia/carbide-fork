/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::computer_system::{BootOptionReference, ComputerSystem};
use nv_redfish::core::{Bmc, EntityTypeRef, ModificationResponse, ODataId};
use nv_redfish::schema::boot_option::BootOption as BootOptionSchema;
use nv_redfish::schema::computer_system::BootUpdate;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
struct BootOptionInfo {
    reference: String,
    display_name: Option<String>,
    uefi_device_path: Option<String>,
    enabled: Option<bool>,
}

impl BootOptionInfo {
    /// The human-readable text a boot entry is matched on.
    fn description(&self) -> String {
        format!(
            "{} {}",
            self.display_name.as_deref().unwrap_or_default(),
            self.uefi_device_path.as_deref().unwrap_or_default()
        )
    }

    fn mentions_any(&self, needles: &[&str]) -> bool {
        let description = self.description().to_ascii_lowercase();
        needles.iter().any(|needle| description.contains(needle))
    }
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn selector_parts(selector: &BootInterfaceSelector) -> (Option<String>, Option<&str>) {
    match selector {
        BootInterfaceSelector::Mac(mac) => (Some(normalized(&mac.to_string())), None),
        BootInterfaceSelector::InterfaceId(interface_id) => (None, Some(interface_id)),
        BootInterfaceSelector::Pair {
            mac_address,
            interface_id,
        } => (
            Some(normalized(&mac_address.to_string())),
            Some(interface_id),
        ),
    }
}

fn boot_option_matches(option: &BootOptionInfo, selector: &BootInterfaceSelector) -> bool {
    selector_matches(&option.description(), selector)
}

/// Reports whether a boot entry's descriptive text names the selected interface.
pub(super) fn selector_matches(text: &str, selector: &BootInterfaceSelector) -> bool {
    let haystack = normalized(text);
    let (mac, interface_id) = selector_parts(selector);
    mac.as_ref().is_none_or(|mac| haystack.contains(mac))
        && interface_id
            .map(normalized)
            .is_none_or(|interface_id| haystack.contains(&interface_id))
}

fn is_network(option: &BootOptionInfo) -> bool {
    option.mentions_any(&["http", "pxe", "network", "ethernet"])
}

fn is_disk(option: &BootOptionInfo) -> bool {
    option.mentions_any(&["hdd", "hard drive", "harddisk", "nvme", "ubuntu", "os boot"])
}

fn boot_order_status(
    order: &[String],
    options: &[BootOptionInfo],
    selector: &BootInterfaceSelector,
) -> BootOrderStatus {
    let selected = options
        .iter()
        .find(|option| boot_option_matches(option, selector));
    let selected_reference = selected.map(|option| option.reference.as_str());
    let first = order.first().map(|entry| {
        entry
            .split_once(": ")
            .map_or(entry.as_str(), |(reference, _)| reference)
    });
    BootOrderStatus {
        boot_interface_first: selected_reference.is_some() && first == selected_reference,
        disk_enabled: options
            .iter()
            .any(|option| is_disk(option) && option.enabled.unwrap_or(false)),
        other_network_options_disabled: options.iter().all(|option| {
            !is_network(option)
                || Some(option.reference.as_str()) == selected_reference
                || option.enabled == Some(false)
        }),
    }
}

fn configured_order<'a>(
    order: &[String],
    options: &'a [BootOptionInfo],
    selector: &BootInterfaceSelector,
) -> Result<(Vec<String>, &'a BootOptionInfo), PlatformError> {
    let target = options
        .iter()
        .find(|option| boot_option_matches(option, selector))
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: "HTTP boot option for selected interface".to_string(),
        })?;
    let mut configured = order.to_vec();
    configured.retain(|entry| {
        entry
            .split_once(": ")
            .map_or(entry.as_str(), |(reference, _)| reference)
            != target.reference
    });
    configured.insert(0, target.reference.clone());
    Ok((configured, target))
}

async fn state<'c, B: Bmc>(
    cx: &'c OpCx<'_, B>,
) -> Result<(&'c ComputerSystem<B>, Vec<String>, Vec<BootOptionInfo>), PlatformError> {
    let system = cx.system()?;
    let order = system
        .boot_order()
        .ok_or(PlatformError::Unsupported)?
        .into_iter()
        .map(|reference| reference.inner().to_string())
        .collect();
    let collection = system
        .boot_options()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let options = collection
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .map(|option| BootOptionInfo {
            reference: option.boot_reference().inner().to_string(),
            display_name: option.display_name().map(|value| value.inner().to_string()),
            uefi_device_path: option
                .uefi_device_path()
                .map(|value| value.inner().to_string()),
            enabled: option.enabled(),
        })
        .collect();
    Ok((system, order, options))
}

#[derive(Serialize)]
struct BootOptionEnabled {
    #[serde(rename = "BootOptionEnabled")]
    enabled: bool,
}

async fn set_option_enabled<B: Bmc>(
    cx: &OpCx<'_, B>,
    system: &ComputerSystem<B>,
    option: &BootOptionInfo,
    enabled: bool,
) -> Result<(), PlatformError> {
    let id = if option.reference.starts_with('/') {
        ODataId::from(option.reference.clone())
    } else {
        ODataId::from(format!(
            "{}/BootOptions/{}",
            system.raw().odata_id().to_string().trim_end_matches('/'),
            option.reference
        ))
    };
    let resource = cx
        .bmc()
        .get::<BootOptionSchema>(&id)
        .await
        .map_err(|error| cx.map_bmc_error(error))?;
    match cx
        .patch_id(&id, resource.etag(), &BootOptionEnabled { enabled })
        .await?
    {
        ModificationResponse::Task(_) => Err(PlatformError::Unsupported),
        ModificationResponse::Entity(_) | ModificationResponse::Empty => Ok(()),
    }
}

pub(crate) async fn status<B: Bmc>(
    cx: &OpCx<'_, B>,
    selector: &BootInterfaceSelector,
) -> Result<BootOrderStatus, PlatformError> {
    let (_, order, options) = state(cx).await?;
    Ok(boot_order_status(&order, &options, selector))
}

pub(crate) async fn configure<B: Bmc>(
    cx: &OpCx<'_, B>,
    selector: &BootInterfaceSelector,
) -> Result<DriverOutcome, PlatformError> {
    let (system, order, options) = state(cx).await?;
    if boot_order_status(&order, &options, selector).is_configured() {
        return Ok(DriverOutcome::complete());
    }
    let (configured, selected) = configured_order(&order, &options, selector)?;
    for option in &options {
        let desired = if option.reference == selected.reference {
            true
        } else if is_network(option) {
            false
        } else {
            option.enabled.unwrap_or(true)
        };
        if option.enabled != Some(desired) {
            set_option_enabled(cx, system, option, desired).await?;
        }
    }
    system
        .set_boot_order(
            configured
                .into_iter()
                .map(BootOptionReference::new)
                .collect(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

pub(crate) async fn set_standard_override<B: Bmc>(
    cx: &OpCx<'_, B>,
    override_setting: &BootUpdate,
) -> Result<DriverOutcome, PlatformError> {
    #[derive(Serialize)]
    struct BootPayload<'a> {
        #[serde(rename = "Boot")]
        boot: &'a BootUpdate,
    }

    // A one-time override belongs on the live system resource; the pending
    // settings object is for staged BIOS changes.
    let system = cx.system()?;
    let raw = system.raw();
    cx.patch_id(
        raw.odata_id(),
        raw.etag(),
        &BootPayload {
            boot: override_setting,
        },
    )
    .await
    .map(DriverOutcome::from)
}

/// Standard boot-option and boot-order policy using advertised BootOptions.
pub(crate) struct StandardBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for StandardBootOrder {
    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        status(cx, selector).await
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
        configure(cx, selector).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector() -> BootInterfaceSelector {
        BootInterfaceSelector::Mac("b8:e9:24:17:6d:72".parse().unwrap())
    }

    fn option(reference: &str, name: &str, enabled: bool) -> BootOptionInfo {
        BootOptionInfo {
            reference: reference.to_string(),
            display_name: Some(name.to_string()),
            uefi_device_path: None,
            enabled: Some(enabled),
        }
    }

    #[test]
    fn matching_accepts_boot_names_and_separator_variants() {
        for name in [
            "UEFI HTTP IPv4 Network Adapter - B8:E9:24:17:6D:72",
            "UEFI HTTPv4 (MAC:B8E924176D72)",
            "HTTP IPv4 Adapter b8-e9-24-17-6d-72",
        ] {
            assert!(boot_option_matches(
                &option("Boot0001", name, true),
                &selector()
            ));
        }
    }

    #[test]
    fn policy_requires_all_three_conditions() {
        let options = vec![
            option("Boot0001", "UEFI HTTPv4 (MAC:B8E924176D72)", true),
            option("Boot0002", "UEFI PXE IPv4 other adapter", false),
            option("Boot0003", "NVMe OS Boot", true),
        ];
        assert!(
            boot_order_status(&["Boot0001".to_string()], &options, &selector()).is_configured()
        );
    }

    #[test]
    fn configured_order_promotes_bare_reference() {
        let options = vec![
            option("Boot0001", "NVMe OS Boot", true),
            option("Boot0002", "UEFI HTTPv4 Network B8:E9:24:17:6D:72", true),
        ];
        assert_eq!(
            configured_order(
                &["Boot0001".to_string(), "Boot0002: HTTP".to_string()],
                &options,
                &selector(),
            )
            .expect("selected option exists")
            .0,
            vec!["Boot0002".to_string(), "Boot0001".to_string()]
        );
    }
}
