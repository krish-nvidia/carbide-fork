/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::computer_system::BootOptionReference;
use nv_redfish::core::Bmc;
use nv_redfish::oem::lenovo::boot_manager::{
    BootOrderKind, LenovoBootManager, LenovoBootManagerCollection, LenovoBootManagerUpdate,
};
use serde_json::{Map, Value};

use crate::boot_order::standard::StandardBootOrder;
use crate::resources::{bios_attributes, patch_bios_attributes, selected_bios};

/// Lenovo XCC boot behavior.
///
/// XCC groups boot options by device class: the system boot order puts the
/// `Network` group first, and the adapter is ordered inside the OEM
/// `NetworkBootOrder`, or through the `BootOrder_NetworkPriority_*` BIOS
/// attributes on firmware without the OEM boot settings. XCC can drop
/// `Hard Disk` from its general order, leaving the installed OS unbootable, so
/// configuring restores it.
pub(crate) struct XccBootOrder;

const NETWORK: &str = "Network";
const HARD_DISK: &str = "Hard Disk";
const NETWORK_PRIORITY: &str = "BootOrder_NetworkPriority_";
/// The network priority attributes XCC exposes.
const NETWORK_PRIORITY_SLOTS: std::ops::RangeInclusive<u32> = 1..=10;

/// The selected interface's MAC, upper-case with colons as XCC spells it.
async fn boot_interface_mac<B: Bmc>(
    cx: &OpCx<'_, B>,
    selector: &BootInterfaceSelector,
) -> Result<String, PlatformError> {
    let interface_id = match selector {
        BootInterfaceSelector::Mac(mac)
        | BootInterfaceSelector::Pair {
            mac_address: mac, ..
        } => {
            return Ok(mac.to_string().to_ascii_uppercase());
        }
        BootInterfaceSelector::InterfaceId(interface_id) => interface_id,
    };
    let interfaces = cx
        .system()?
        .ethernet_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?;
    interfaces
        .iter()
        .find(|interface| interface.raw().id == *interface_id)
        .and_then(|interface| interface.mac_address())
        .map(|mac| mac.as_str().to_ascii_uppercase())
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: format!("MAC address of interface {interface_id}"),
        })
}

/// Whether an OEM network boot entry is the HTTP IPv4 option of the adapter
/// with `mac`, as in `UEFI: SLOT2 (31/0/0) HTTP IPv4  Nvidia Network Adapter - A0:88:C2:08:53:C4`.
fn names_adapter(entry: &str, mac: &str) -> bool {
    let suffix = format!(" - {mac}");
    ["HTTP IPv4  Mellanox", "HTTP IPv4  Nvidia"]
        .iter()
        .any(|prefix| {
            entry
                .find(prefix)
                .is_some_and(|start| entry[start..].to_ascii_uppercase().contains(&suffix))
        })
}

/// Whether a network priority attribute value is the HTTP IPv4 option of the
/// adapter with `mac`, as in `Slot5Port1HTTPv4NvidiaNetworkAdapter_B8_E9_24_18_42_52`.
fn priority_names_adapter(value: &str, mac: &str) -> bool {
    value.contains("HTTPv4") && value.to_ascii_uppercase().contains(&mac.replace(':', "_"))
}

async fn boot_settings<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<LenovoBootManagerCollection<B>>, PlatformError> {
    cx.system()?
        .oem_lenovo()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .boot_settings()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

async fn boot_order<B: Bmc>(
    cx: &OpCx<'_, B>,
    settings: &LenovoBootManagerCollection<B>,
    kind: BootOrderKind,
) -> Result<LenovoBootManager<B>, PlatformError> {
    settings
        .boot_order(kind)
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("XCC boot settings do not list the {kind:?} boot order"),
        })
}

async fn set_next<B: Bmc>(
    cx: &OpCx<'_, B>,
    order: &LenovoBootManager<B>,
    next: Vec<String>,
) -> Result<DriverOutcome, PlatformError> {
    order
        .update(
            &LenovoBootManagerUpdate::builder()
                .with_boot_order_next(next)
                .build(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// `next` with `entry` first: swapped with the first entry when listed,
/// otherwise inserted at the front.
fn with_first(mut next: Vec<String>, entry: &str) -> Vec<String> {
    match next.iter().position(|candidate| candidate == entry) {
        Some(position) => next.swap(0, position),
        None => next.insert(0, entry.to_string()),
    }
    next
}

/// The adapter's network boot entry and the entry currently first, from the OEM
/// network order or, without OEM boot settings, the BIOS network priorities.
async fn expected_and_first_network_option<B: Bmc>(
    cx: &OpCx<'_, B>,
    settings: Option<&LenovoBootManagerCollection<B>>,
    mac: &str,
) -> Result<(Option<String>, Option<String>), PlatformError> {
    if let Some(settings) = settings {
        let network = boot_order(cx, settings, BootOrderKind::Network).await?;
        let expected = network
            .supported()
            .unwrap_or_default()
            .iter()
            .find(|entry| names_adapter(entry, mac))
            .cloned();
        let first = network.next().unwrap_or_default().first().cloned();
        return Ok((expected, first));
    }
    let attributes = bios_attributes(&selected_bios(cx).await?.raw());
    let priority = |slot: u32| {
        attributes
            .get(&format!("{NETWORK_PRIORITY}{slot}"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let expected = NETWORK_PRIORITY_SLOTS
        .filter_map(priority)
        .find(|value| priority_names_adapter(value, mac));
    Ok((expected, priority(1)))
}

/// Puts the adapter's entry first in the OEM network order, restoring
/// `Hard Disk` in the general order first.
async fn configure_oem<B: Bmc>(
    cx: &OpCx<'_, B>,
    settings: &LenovoBootManagerCollection<B>,
    mac: &str,
) -> Result<DriverOutcome, PlatformError> {
    let mut outcome = DriverOutcome::complete();
    let general = boot_order(cx, settings, BootOrderKind::General).await?;
    let general_next = general.next().unwrap_or_default().to_vec();
    if !general_next.iter().any(|entry| entry == HARD_DISK) {
        let mut next = general_next;
        next.insert(next.len().min(1), HARD_DISK.to_string());
        outcome = outcome.merge(set_next(cx, &general, next).await?);
    }

    let network = boot_order(cx, settings, BootOrderKind::Network).await?;
    let adapter = network
        .supported()
        .unwrap_or_default()
        .iter()
        .find(|entry| names_adapter(entry, mac))
        .cloned()
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: format!("XCC NetworkBootOrder HTTP IPv4 option for {mac}"),
        })?;
    let network_next = network.next().unwrap_or_default().to_vec();
    if network_next.first() != Some(&adapter) {
        outcome = outcome.merge(set_next(cx, &network, with_first(network_next, &adapter)).await?);
    }
    Ok(outcome)
}

/// Swaps the adapter's BIOS network priority into the first slot.
async fn configure_bios_priority<B: Bmc>(
    cx: &OpCx<'_, B>,
    mac: &str,
) -> Result<DriverOutcome, PlatformError> {
    let attributes = bios_attributes(&selected_bios(cx).await?.raw());
    let priority = |slot: u32| {
        attributes
            .get(&format!("{NETWORK_PRIORITY}{slot}"))
            .and_then(Value::as_str)
    };
    let (slot, adapter) = NETWORK_PRIORITY_SLOTS
        .filter_map(|slot| priority(slot).map(|value| (slot, value)))
        .find(|(_, value)| priority_names_adapter(value, mac))
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: format!("{NETWORK_PRIORITY}* HTTPv4 option for {mac}"),
        })?;
    if slot == 1 {
        return Ok(DriverOutcome::complete());
    }
    let mut swapped = Map::new();
    swapped.insert(format!("{NETWORK_PRIORITY}1"), adapter.into());
    if let Some(first) = priority(1) {
        swapped.insert(format!("{NETWORK_PRIORITY}{slot}"), first.into());
    }
    patch_bios_attributes(cx, Value::Object(swapped)).await
}

/// The system boot order with the `Network` group first, or `None` when it
/// already is.
async fn network_group_first<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<Vec<String>>, PlatformError> {
    let system = cx.system()?;
    let order: Vec<String> = system
        .boot_order()
        .unwrap_or_default()
        .into_iter()
        .map(|reference| reference.inner().to_string())
        .collect();
    let network = system
        .boot_options()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .find(|option| option.raw().name == NETWORK)
        .map(|option| option.boot_reference().inner().to_string())
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: format!("{NETWORK} boot option"),
        })?;
    if order.first() == Some(&network) {
        return Ok(None);
    }
    Ok(Some(with_first(order, &network)))
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for XccBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        let mac = boot_interface_mac(cx, selector).await?;
        let network_first = match network_group_first(cx).await {
            Ok(order) => order.is_none(),
            Err(PlatformError::MissingBootOption { .. }) => false,
            Err(error) => return Err(error),
        };
        let settings = boot_settings(cx).await?;
        let (expected, first) =
            expected_and_first_network_option(cx, settings.as_ref(), &mac).await?;
        let disk_enabled = match &settings {
            Some(settings) => boot_order(cx, settings, BootOrderKind::General)
                .await?
                .next()
                .unwrap_or_default()
                .iter()
                .any(|entry| entry == HARD_DISK),
            None => true,
        };
        Ok(BootOrderStatus {
            boot_interface_first: network_first && expected.is_some() && expected == first,
            disk_enabled,
            // The Network group holds every adapter; ordering the selected one
            // first within it is the only network policy XCC applies.
            other_network_options_disabled: true,
        })
    }

    async fn configure(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        if self.status(cx, selector).await?.is_configured() {
            return Ok(DriverOutcome::complete());
        }
        let mac = boot_interface_mac(cx, selector).await?;
        let settings = boot_settings(cx).await?;
        let mut outcome = DriverOutcome::complete();
        match (network_group_first(cx).await, &settings) {
            (Ok(Some(order)), _) => {
                outcome = outcome.merge(
                    cx.system()?
                        .set_boot_order(order.into_iter().map(BootOptionReference::new).collect())
                        .await
                        .map(DriverOutcome::from)
                        .map_err(|error| cx.map_redfish_error(error))?,
                );
            }
            (Ok(None), _) => {}
            // Without a Network boot option the network group is missing from
            // the OEM general order, which leaves the network order empty.
            (Err(PlatformError::MissingBootOption { .. }), Some(settings)) => {
                let general = boot_order(cx, settings, BootOrderKind::General).await?;
                let next = general.next().unwrap_or_default().to_vec();
                outcome = outcome.merge(set_next(cx, &general, with_first(next, NETWORK)).await?);
            }
            (Err(error), _) => return Err(error),
        }
        let adapter = match &settings {
            Some(settings) => configure_oem(cx, settings, &mac).await?,
            None => configure_bios_priority(cx, &mac).await?,
        };
        Ok(outcome.merge(adapter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_entries_are_the_http_ipv4_options_naming_the_mac() {
        let mac = "A0:88:C2:08:53:C4";
        for entry in [
            "UEFI:   SLOT2 (31/0/0) HTTP IPv4  Nvidia Network Adapter - A0:88:C2:08:53:C4",
            "UEFI:   SLOT 1 (41/0/0) HTTP IPv4  Nvidia BlueField-3 VPI QSFP112 2P 200G PCIe Gen5 x16 - A0:88:C2:08:53:C4",
            "UEFI:   SLOT1 (4B/0/0) HTTP IPv4  Mellanox Network Adapter - a0:88:c2:08:53:c4",
        ] {
            assert!(names_adapter(entry, mac), "{entry}");
        }
        for entry in [
            "UEFI:   SLOT2 (31/0/0) PXE IPv4  Nvidia Network Adapter - A0:88:C2:08:53:C4",
            "UEFI:   SLOT2 (31/0/0) HTTP IPv4  Nvidia Network Adapter - A0:88:C2:08:53:C5",
        ] {
            assert!(!names_adapter(entry, mac), "{entry}");
        }
        assert!(priority_names_adapter(
            "Slot5Port1HTTPv4NvidiaNetworkAdapter_A0_88_C2_08_53_C4",
            mac
        ));
        assert!(!priority_names_adapter(
            "Slot5Port1PXEv4NvidiaNetworkAdapter_A0_88_C2_08_53_C4",
            mac
        ));
    }
}
