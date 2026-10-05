/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Boot-order reads and writes shared by the boot-order drivers.
//!
//! Each driver finds the selected interface's boot option and writes the boot
//! order the way its platform's hardware-verified integration does; these are
//! the pieces they have in common.

use bmc_platform::{BootInterfaceSelector, DriverOutcome, OpCx, PlatformError};
use nv_redfish::computer_system::{BootOption, ComputerSystem, ComputerSystemUpdate};
use nv_redfish::core::{Bmc, ModificationResponse, ODataETag, ODataId};
use nv_redfish::schema::computer_system::{
    BootSource, BootSourceOverrideEnabled, BootSourceOverrideMode, BootUpdate,
};
use serde_json::Value;

/// The selected system's boot options, in collection order.
pub(crate) async fn boot_options<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Vec<BootOption<B>>, PlatformError> {
    cx.system()
        .await?
        .boot_options()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

/// The option's display name; empty when the BMC omits it.
pub(crate) fn display_name<B: Bmc>(option: &BootOption<B>) -> &str {
    option.display_name().map_or("", |name| *name.inner())
}

/// The option's `BootOptionReference`, or its id when the BMC omits it.
pub(crate) fn reference<B: Bmc>(option: &BootOption<B>) -> &str {
    option.boot_reference().inner()
}

/// The option's `Alias`.
pub(crate) fn alias<B: Bmc>(option: &BootOption<B>) -> Option<BootSource> {
    option.raw().alias.flatten()
}

/// Whether the option is the one `entry` of a `BootOrder` names, by
/// reference or resource id.
fn names_option<B: Bmc>(entry: &str, option: &BootOption<B>) -> bool {
    let entry = entry_reference(entry);
    reference(option) == entry || option.raw().id == entry
}

/// The selected system's `Boot.BootOrder` as reported; empty when absent.
pub(crate) fn boot_order<B: Bmc>(system: &ComputerSystem<B>) -> Vec<String> {
    system
        .boot_order()
        .unwrap_or_default()
        .into_iter()
        .map(|reference| reference.inner().to_string())
        .collect()
}

/// The boot option reference a `BootOrder` entry names; some firmware reports
/// entries as `Boot0001: <display name>`.
fn entry_reference(entry: &str) -> &str {
    entry
        .split_once(": ")
        .map_or(entry, |(reference, _)| reference)
}

/// Moves the entry naming `reference` to the front of `order`, keeping the
/// entry as the BMC reports it; `false` when no entry names it.
pub(crate) fn promote(order: &mut Vec<String>, reference: &str) -> bool {
    let Some(position) = order
        .iter()
        .position(|entry| entry_reference(entry) == reference)
    else {
        return false;
    };
    let entry = order.remove(position);
    order.retain(|entry| entry_reference(entry) != reference);
    order.insert(0, entry);
    true
}

/// `order` with the entry naming `reference` first, inserting `reference`
/// when no entry names it.
fn with_first(mut order: Vec<String>, reference: &str) -> Vec<String> {
    if !promote(&mut order, reference) {
        order.insert(0, reference.to_string());
    }
    order
}

/// Whether `order` starts with the entry naming `reference`.
pub(crate) fn is_first(order: &[String], reference: &str) -> bool {
    order
        .first()
        .is_some_and(|entry| entry_reference(entry) == reference)
}

/// Writes `order` with the entry naming `reference` first to the system
/// resource at `uri`, unless `order` already starts with it.
pub(crate) async fn put_first<B: Bmc>(
    cx: &OpCx<'_, B>,
    uri: &ODataId,
    order: Vec<String>,
    reference: &str,
) -> Result<DriverOutcome, PlatformError> {
    if is_first(&order, reference) {
        return Ok(DriverOutcome::complete());
    }
    write_boot_order(cx, uri, with_first(order, reference)).await
}

/// The boot option `entry` of a `BootOrder` names.
pub(crate) fn listed_option<'a, B: Bmc>(
    entry: &str,
    options: &'a [BootOption<B>],
) -> Result<&'a BootOption<B>, PlatformError> {
    options
        .iter()
        .find(|option| names_option(entry, option))
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("BootOrder lists {entry}, which is not a boot option"),
        })
}

/// The boot options `order` lists, in order.
pub(crate) fn listed_options<'a, B: Bmc>(
    order: &[String],
    options: &'a [BootOption<B>],
) -> Result<Vec<&'a BootOption<B>>, PlatformError> {
    order
        .iter()
        .map(|entry| listed_option(entry, options))
        .collect()
}

/// The boot options `order` lists, in order, with every option `matches`
/// accepts moved to the front, the later ones first.
pub(crate) fn matching_first<'a, B: Bmc>(
    order: &[String],
    options: &'a [BootOption<B>],
    matches: impl Fn(&BootOption<B>) -> bool,
) -> Result<Vec<&'a BootOption<B>>, PlatformError> {
    let mut ordered = Vec::with_capacity(order.len());
    for option in listed_options(order, options)? {
        if matches(option) {
            ordered.insert(0, option);
        } else {
            ordered.push(option);
        }
    }
    Ok(ordered)
}

/// The selected interface's MAC: the selector's own, or the one the system's
/// Ethernet interface with the selector's id reports.
pub(crate) async fn boot_interface_mac<B: Bmc>(
    cx: &OpCx<'_, B>,
    selector: &BootInterfaceSelector,
) -> Result<String, PlatformError> {
    let interface_id = match selector {
        BootInterfaceSelector::Mac(mac)
        | BootInterfaceSelector::Pair {
            mac_address: mac, ..
        } => return Ok(mac.to_string()),
        BootInterfaceSelector::InterfaceId(interface_id) => interface_id,
    };
    let interfaces = cx
        .system()
        .await?
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
        .map(|mac| mac.as_str().to_string())
        .filter(|mac| !mac.is_empty())
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("EthernetInterface {interface_id} reports no MACAddress"),
        })
}

/// The selected system's resource, or the resource at `suffix` beneath it,
/// such as its `Settings`, `SD` or `Pending` object.
pub(crate) async fn system_uri<B: Bmc>(
    cx: &OpCx<'_, B>,
    suffix: Option<&str>,
) -> Result<ODataId, PlatformError> {
    let system = cx.system().await?.raw().odata_id.to_string();
    Ok(ODataId::from(match suffix {
        Some(suffix) => format!("{system}/{suffix}"),
        None => system,
    }))
}

/// Writes `boot` to the system resource at `uri`, with `If-Match` set to
/// `etag` or `*`.
pub(crate) async fn patch_boot<B: Bmc>(
    cx: &OpCx<'_, B>,
    uri: &ODataId,
    boot: BootUpdate,
    etag: Option<&ODataETag>,
) -> Result<ModificationResponse<Value>, PlatformError> {
    cx.bmc()
        .update(
            uri,
            etag,
            &ComputerSystemUpdate::builder().with_boot(boot).build(),
        )
        .await
        .map_err(|error| cx.map_bmc_error(error))
}

/// Writes `order` as the `Boot.BootOrder` of the system resource at `uri`.
pub(crate) async fn write_boot_order<B: Bmc>(
    cx: &OpCx<'_, B>,
    uri: &ODataId,
    order: Vec<String>,
) -> Result<DriverOutcome, PlatformError> {
    let boot = BootUpdate::builder().with_boot_order(order).build();
    patch_boot(cx, uri, boot, None)
        .await
        .map(DriverOutcome::from)
}

/// Writes the override fields of `setting` to the system resource at `uri`:
/// target, enablement, the mode (UEFI when `uefi_by_default` and the caller
/// left it unset), and the HTTP boot URI.
pub(crate) async fn write_override<B: Bmc>(
    cx: &OpCx<'_, B>,
    uri: &ODataId,
    setting: &BootUpdate,
    uefi_by_default: bool,
    etag: Option<&ODataETag>,
) -> Result<DriverOutcome, PlatformError> {
    let mode = match setting.boot_source_override_mode {
        None if uefi_by_default => Some(BootSourceOverrideMode::Uefi),
        mode => mode,
    };
    let boot = BootUpdate {
        boot_source_override_target: Some(
            setting
                .boot_source_override_target
                .ok_or(PlatformError::Unsupported)?,
        ),
        boot_source_override_enabled: Some(
            setting
                .boot_source_override_enabled
                .ok_or(PlatformError::Unsupported)?,
        ),
        boot_source_override_mode: mode,
        http_boot_uri: setting.http_boot_uri.clone(),
        ..BootUpdate::default()
    };
    patch_boot(cx, uri, boot, etag)
        .await
        .map(DriverOutcome::from)
}

/// The device a request to boot one of the network, HTTP or disk devices
/// persistently names: a `Continuous` override with no mode or HTTP boot URI.
/// Platforms reorder their boot order for it rather than set an override.
pub(crate) fn persistent_device(setting: &BootUpdate) -> Option<BootSource> {
    device_for(setting, BootSourceOverrideEnabled::Continuous)
}

/// The device a request to boot one of the network, HTTP or disk devices
/// once names: a `Once` override with no mode or HTTP boot URI.
pub(crate) fn one_time_device(setting: &BootUpdate) -> Option<BootSource> {
    device_for(setting, BootSourceOverrideEnabled::Once)
}

fn device_for(setting: &BootUpdate, enabled: BootSourceOverrideEnabled) -> Option<BootSource> {
    if setting.boot_source_override_enabled != Some(enabled)
        || setting.boot_source_override_mode.is_some()
        || setting.http_boot_uri.is_some()
    {
        return None;
    }
    setting.boot_source_override_target.filter(|target| {
        matches!(
            target,
            BootSource::Pxe | BootSource::Hdd | BootSource::UefiHttp
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_bare_continuous_device_override_reorders_the_boot_order() {
        let setting =
            |target, enabled, mode: Option<BootSourceOverrideMode>, uri: Option<&str>| BootUpdate {
                boot_source_override_target: Some(target),
                boot_source_override_enabled: Some(enabled),
                boot_source_override_mode: mode,
                http_boot_uri: uri.map(str::to_string),
                ..BootUpdate::default()
            };
        let continuous = BootSourceOverrideEnabled::Continuous;
        let cases = [
            (
                "continuous disk",
                setting(BootSource::Hdd, continuous, None, None),
                Some(BootSource::Hdd),
            ),
            (
                "once",
                setting(BootSource::Pxe, BootSourceOverrideEnabled::Once, None, None),
                None,
            ),
            (
                "with a mode",
                setting(
                    BootSource::Pxe,
                    continuous,
                    Some(BootSourceOverrideMode::Uefi),
                    None,
                ),
                None,
            ),
            (
                "with an HTTP boot URI",
                setting(BootSource::UefiHttp, continuous, None, Some("http://boot")),
                None,
            ),
            (
                "another device",
                setting(BootSource::Cd, continuous, None, None),
                None,
            ),
        ];
        for (name, setting, expected) in cases {
            assert_eq!(persistent_device(&setting), expected, "{name}");
        }
    }

    #[test]
    fn promotion_keeps_the_reported_entry_and_drops_duplicates() {
        let order = |entries: &[&str]| entries.iter().map(|entry| (*entry).to_string()).collect();
        assert_eq!(
            with_first(
                order(&["Boot0001", "Boot0002: UEFI HTTPv4", "Boot0002"]),
                "Boot0002"
            ),
            order(&["Boot0002: UEFI HTTPv4", "Boot0001"])
        );
        assert_eq!(
            with_first(order(&["Boot0001"]), "Boot0003"),
            order(&["Boot0003", "Boot0001"])
        );
    }
}
