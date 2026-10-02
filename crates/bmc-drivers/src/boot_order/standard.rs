/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish boot control.

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::computer_system::{
    BootOption, BootOptionReference, BootOptionUpdate, ComputerSystem, ComputerSystemUpdate,
};
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::{BootSourceOverrideMode, BootUpdate};

/// Standard boot-option and boot-order policy using advertised BootOptions.
pub(crate) struct StandardBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for StandardBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        self
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        let (_, order, options, _, interfaces) = state(cx).await?;
        Ok(boot_order_status(&order, &options, &interfaces, selector))
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        set_override(cx, override_setting).await
    }

    async fn configure(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        configure(cx, selector).await
    }
}

/// Places a one-time override on the live system resource; the pending
/// settings object is for staged BIOS changes.
async fn set_override<B: Bmc>(
    cx: &OpCx<'_, B>,
    override_setting: &BootUpdate,
) -> Result<DriverOutcome, PlatformError> {
    let system = cx.system()?;
    let body = ComputerSystemUpdate::builder()
        .with_boot(boot_source_override(override_setting))
        .build();
    system
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Places an override through the system's pending-settings resource, booting
/// UEFI when the caller left the mode unset.
pub(super) async fn settings_object_override<B: Bmc>(
    cx: &OpCx<'_, B>,
    setting: &BootUpdate,
) -> Result<DriverOutcome, PlatformError> {
    cx.system()?
        .set_boot_source_override(
            setting
                .boot_source_override_target
                .ok_or(PlatformError::Unsupported)?,
            setting
                .boot_source_override_enabled
                .ok_or(PlatformError::Unsupported)?,
            Some(
                setting
                    .boot_source_override_mode
                    .unwrap_or(BootSourceOverrideMode::Uefi),
            ),
            setting.http_boot_uri.clone(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// `setting` with the boot mode UEFI when the caller left it unset.
pub(super) fn uefi_by_default(setting: &BootUpdate) -> BootUpdate {
    BootUpdate {
        boot_source_override_mode: setting
            .boot_source_override_mode
            .or(Some(BootSourceOverrideMode::Uefi)),
        ..boot_source_override(setting)
    }
}

/// The boot-source override fields of `setting`: target, enablement, mode,
/// and HTTP boot URI.
fn boot_source_override(setting: &BootUpdate) -> BootUpdate {
    BootUpdate {
        boot_source_override_target: setting.boot_source_override_target,
        boot_source_override_enabled: setting.boot_source_override_enabled,
        boot_source_override_mode: setting.boot_source_override_mode,
        http_boot_uri: setting.http_boot_uri.clone(),
        ..BootUpdate::default()
    }
}

/// Promotes the selected interface's boot option, disables the other network
/// options, enables the disks, and writes the resulting `BootOrder`.
async fn configure<B: Bmc>(
    cx: &OpCx<'_, B>,
    selector: &BootInterfaceSelector,
) -> Result<DriverOutcome, PlatformError> {
    let (system, order, options, resources, interfaces) = state(cx).await?;
    if boot_order_status(&order, &options, &interfaces, selector).is_configured() {
        return Ok(DriverOutcome::complete());
    }
    let (configured, selected) = configured_order(&order, &options, &interfaces, selector)?;
    for (option, resource) in options.iter().zip(&resources) {
        let desired = if option.reference == selected.reference {
            true
        } else if is_network(option) {
            false
        } else if is_disk(option) {
            true
        } else {
            option.enabled.unwrap_or(true)
        };
        if option.enabled != Some(desired) {
            set_option_enabled(cx, resource, desired).await?;
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

/// Reports whether a boot entry's descriptive text names the selected interface.
pub(super) fn selector_matches(text: &str, selector: &BootInterfaceSelector) -> bool {
    let haystack = normalized(text);
    let (mac, interface_id) = selector_parts(selector);
    mac.as_ref().is_none_or(|mac| haystack.contains(mac))
        && interface_id
            .map(normalized)
            .is_none_or(|interface_id| haystack.contains(&interface_id))
}

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

/// A system Ethernet interface as boot-option matching reads it; the MAC is
/// upper-case hex without separators, as UEFI device paths spell it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct InterfaceInfo {
    id: String,
    mac: Option<String>,
    uefi_device_path: Option<String>,
}

/// `mac` as upper-case hex without separators.
fn mac_hex(mac: &str) -> String {
    mac.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|digit| digit.to_ascii_uppercase())
        .collect()
}

/// The selected interface's IPv4 boot option, preferring UEFI HTTP over PXE.
///
/// The option's UEFI device path extends the device path of the system
/// Ethernet interface the selector names; when that interface reports no
/// device path, the option's path must name the interface's MAC instead.
fn selected_option<'a>(
    options: &'a [BootOptionInfo],
    interfaces: &[InterfaceInfo],
    selector: &BootInterfaceSelector,
) -> Option<&'a BootOptionInfo> {
    let (mac, interface_id) = match selector {
        BootInterfaceSelector::Mac(mac) => (Some(mac_hex(&mac.to_string())), None),
        BootInterfaceSelector::InterfaceId(interface_id) => (None, Some(interface_id.as_str())),
        BootInterfaceSelector::Pair {
            mac_address,
            interface_id,
        } => (
            Some(mac_hex(&mac_address.to_string())),
            Some(interface_id.as_str()),
        ),
    };
    let interface = interfaces
        .iter()
        .find(|interface| mac.is_some() && interface.mac == mac)
        .or_else(|| {
            interfaces
                .iter()
                .find(|interface| Some(interface.id.as_str()) == interface_id)
        });
    let interface_path = interface.and_then(|interface| interface.uefi_device_path.as_deref());
    let mac_node = mac
        .or_else(|| interface.and_then(|interface| interface.mac.clone()))
        .map(|mac| format!("MAC({mac}"));
    let candidates: Vec<&BootOptionInfo> = options
        .iter()
        .filter(|option| {
            option.uefi_device_path.as_deref().is_some_and(|path| {
                path.contains("/IPv4(")
                    && match interface_path {
                        Some(interface_path) => is_uefi_tree_child(interface_path, path),
                        None => mac_node
                            .as_deref()
                            .is_some_and(|node| path.to_ascii_uppercase().contains(node)),
                    }
            })
        })
        .collect();
    candidates
        .iter()
        .find(|option| {
            option
                .uefi_device_path
                .as_deref()
                .is_some_and(|path| path.contains("/Uri("))
        })
        .or_else(|| candidates.first())
        .copied()
}

/// Whether the boot option `child` lies under the device path `parent`.
///
/// HPE reports option paths under `Acpi(0x00168E09,<n>)` where the interface
/// reports `PciRoot(<n>)`.
fn is_uefi_tree_child(parent: &str, child: &str) -> bool {
    const HPE_ACPI_PREFIX: &str = "Acpi(0x00168E09,";
    const PCI_ROOT_PREFIX: &str = "PciRoot(";
    match (
        child.strip_prefix(HPE_ACPI_PREFIX),
        parent.strip_prefix(PCI_ROOT_PREFIX),
    ) {
        (Some(child), Some(parent)) => child.starts_with(parent),
        _ => child.starts_with(parent),
    }
}

fn is_network(option: &BootOptionInfo) -> bool {
    option.mentions_any(&["http", "pxe", "network", "ethernet"])
}

fn is_disk(option: &BootOptionInfo) -> bool {
    option.mentions_any(&[
        "hdd",
        "hard drive",
        "harddisk",
        "disk",
        "ssd",
        "nvme",
        "ubuntu",
        "os boot",
        "/hd(",
        "sata(",
    ])
}

fn boot_order_status(
    order: &[String],
    options: &[BootOptionInfo],
    interfaces: &[InterfaceInfo],
    selector: &BootInterfaceSelector,
) -> BootOrderStatus {
    let selected = selected_option(options, interfaces, selector);
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
    interfaces: &[InterfaceInfo],
    selector: &BootInterfaceSelector,
) -> Result<(Vec<String>, &'a BootOptionInfo), PlatformError> {
    let target = selected_option(options, interfaces, selector).ok_or_else(|| {
        PlatformError::MissingBootOption {
            description: "HTTP boot option for selected interface".to_string(),
        }
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

/// The selected system, its boot order, every boot option both as the policy
/// reads it and as the resource it writes (in the same order), and the
/// system's Ethernet interfaces.
type BootState<'c, B> = (
    &'c ComputerSystem<B>,
    Vec<String>,
    Vec<BootOptionInfo>,
    Vec<BootOption<B>>,
    Vec<InterfaceInfo>,
);

async fn state<'c, B: Bmc>(cx: &'c OpCx<'_, B>) -> Result<BootState<'c, B>, PlatformError> {
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
    let resources = collection
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?;
    let options = resources
        .iter()
        .map(|option| BootOptionInfo {
            reference: option.boot_reference().inner().to_string(),
            display_name: option.display_name().map(|value| value.inner().to_string()),
            uefi_device_path: option
                .uefi_device_path()
                .map(|value| value.inner().to_string()),
            enabled: option.enabled(),
        })
        .collect();
    let interfaces = system
        .ethernet_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?;
    let interfaces = match interfaces {
        Some(collection) => collection
            .members()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .iter()
            .map(|interface| InterfaceInfo {
                id: interface.raw().id.clone(),
                mac: interface.mac_address().map(|mac| mac_hex(mac.as_str())),
                uefi_device_path: interface
                    .uefi_device_path()
                    .map(|path| path.inner().to_string()),
            })
            .collect(),
        None => Vec::new(),
    };
    Ok((system, order, options, resources, interfaces))
}

async fn set_option_enabled<B: Bmc>(
    cx: &OpCx<'_, B>,
    option: &BootOption<B>,
    enabled: bool,
) -> Result<(), PlatformError> {
    let body = BootOptionUpdate::builder()
        .with_boot_option_enabled(enabled)
        .build();
    match option
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))?
    {
        DriverOutcome::Complete { .. } => Ok(()),
        DriverOutcome::Accepted { .. } | DriverOutcome::Blocked { .. } => {
            Err(PlatformError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERFACE_PATH: &str = "PciRoot(0x0)/Pci(0x10,0x0)/Pci(0x0,0x0)";
    const HTTP_PATH: &str = "PciRoot(0x0)/Pci(0x10,0x0)/Pci(0x0,0x0)/MAC(B8E924176D72,0x1)/IPv4(0.0.0.0,0x0,DHCP,0.0.0.0,0.0.0.0,0.0.0.0)/Uri()";
    const PXE_PATH: &str = "PciRoot(0x0)/Pci(0x10,0x0)/Pci(0x0,0x0)/MAC(B8E924176D72,0x1)/IPv4(0.0.0.0,0x0,DHCP,0.0.0.0,0.0.0.0,0.0.0.0)";
    const OTHER_HTTP_PATH: &str = "PciRoot(0x2)/Pci(0x1,0x0)/Pci(0x0,0x0)/MAC(B8E924170000,0x1)/IPv4(0.0.0.0,0x0,DHCP,0.0.0.0,0.0.0.0,0.0.0.0)/Uri()";
    const DISK_PATH: &str = "PciRoot(0x0)/Pci(0x1D,0x0)/Pci(0x0,0x0)/NVMe(0x1,00-00-00-00-00-00-00-00)/HD(1,GPT,0,0x800,0x100000)";

    fn selector() -> BootInterfaceSelector {
        BootInterfaceSelector::Mac("b8:e9:24:17:6d:72".parse().expect("valid MAC"))
    }

    fn option(reference: &str, name: &str, path: &str, enabled: bool) -> BootOptionInfo {
        BootOptionInfo {
            reference: reference.to_string(),
            display_name: Some(name.to_string()),
            uefi_device_path: Some(path.to_string()),
            enabled: Some(enabled),
        }
    }

    fn interface(path: Option<&str>) -> InterfaceInfo {
        InterfaceInfo {
            id: "NIC.Slot.1-1".to_string(),
            mac: Some("B8E924176D72".to_string()),
            uefi_device_path: path.map(str::to_string),
        }
    }

    #[test]
    fn the_interfaces_http_option_is_selected_by_device_path() {
        let options = vec![
            option("Boot0001", "UEFI PXE IPv4", PXE_PATH, true),
            option("Boot0002", "UEFI HTTP IPv4", HTTP_PATH, true),
            option("Boot0003", "UEFI HTTP IPv4", OTHER_HTTP_PATH, true),
        ];
        for interfaces in [
            vec![interface(Some(INTERFACE_PATH))],
            vec![interface(None)],
            vec![],
        ] {
            assert_eq!(
                selected_option(&options, &interfaces, &selector())
                    .map(|option| option.reference.as_str()),
                Some("Boot0002"),
                "{interfaces:?}"
            );
        }
        let hpe = option(
            "Boot0004",
            "UEFI HTTP IPv4",
            "Acpi(0x00168E09,0x0)/Pci(0x10,0x0)/Pci(0x0,0x0)/MAC(B8E924176D72,0x1)/IPv4(0.0.0.0)/Uri()",
            true,
        );
        assert_eq!(
            selected_option(&[hpe], &[interface(Some(INTERFACE_PATH))], &selector())
                .map(|option| option.reference.as_str()),
            Some("Boot0004")
        );
    }

    #[test]
    fn policy_requires_all_three_conditions() {
        let options = vec![
            option("Boot0001", "UEFI HTTP IPv4", HTTP_PATH, true),
            option(
                "Boot0002",
                "UEFI PXE IPv4 other adapter",
                OTHER_HTTP_PATH,
                false,
            ),
            option("Boot0003", "Red Hat Enterprise Linux", DISK_PATH, true),
        ];
        let interfaces = [interface(Some(INTERFACE_PATH))];
        assert!(
            boot_order_status(
                &["Boot0001".to_string()],
                &options,
                &interfaces,
                &selector()
            )
            .is_configured()
        );
    }

    #[test]
    fn configured_order_promotes_bare_reference() {
        let options = vec![
            option("Boot0001", "NVMe OS Boot", DISK_PATH, true),
            option("Boot0002", "UEFI HTTP IPv4", HTTP_PATH, true),
        ];
        assert_eq!(
            configured_order(
                &["Boot0001".to_string(), "Boot0002: HTTP".to_string()],
                &options,
                &[interface(Some(INTERFACE_PATH))],
                &selector(),
            )
            .expect("selected option exists")
            .0,
            vec!["Boot0002".to_string(), "Boot0001".to_string()]
        );
    }
}
