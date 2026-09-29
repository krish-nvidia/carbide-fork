/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::oem::hpe::server_boot_settings::HpeServerBootSettingsUpdate;
use nv_redfish::schema::computer_system::{BootSource, BootUpdate};
use serde_json::json;

use crate::boot_order::standard::StandardBootOrder;
use crate::resources::{patch_bios_attributes, selected_bios};

/// HPE iLO boot behavior.
///
/// iLO rejects `BootSourceOverride` writes. A UEFI HTTP URI is pinned through
/// BIOS attributes; any other override moves the target's category to the
/// front of the persistent boot order.
pub(crate) struct IloBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for IloBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        if let Some(uri) = override_setting.http_boot_uri.as_deref() {
            return patch_bios_attributes(
                cx,
                json!({"UrlBootFile": uri, "PreBootNetwork": "IPv4"}),
            )
            .await;
        }
        let category = match override_setting.boot_source_override_target {
            Some(BootSource::Pxe | BootSource::UefiHttp) => "nic.",
            Some(BootSource::Hdd) => "hd.",
            _ => return Err(PlatformError::Unsupported),
        };
        set_persistent_boot_first(cx, category).await
    }
}

/// Moves every persistent boot entry naming `category` to the front.
async fn set_persistent_boot_first<B: Bmc>(
    cx: &OpCx<'_, B>,
    category: &str,
) -> Result<DriverOutcome, PlatformError> {
    let boot = selected_bios(cx)
        .await?
        .oem_hpe()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .boot_settings()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let order =
        boot.persistent_boot_config_order()
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: "HPE boot settings report no PersistentBootConfigOrder".to_string(),
            })?;
    let order = category_first(order, category);
    boot.settings()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .update(
            &HpeServerBootSettingsUpdate::builder()
                .with_persistent_boot_config_order(order)
                .build(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Each entry naming `category` is inserted at the front as it is met, so
/// matching entries end up first in reverse order.
fn category_first(order: &[String], category: &str) -> Vec<String> {
    let mut reordered = Vec::with_capacity(order.len());
    for entry in order {
        if entry.to_ascii_lowercase().contains(category) {
            reordered.insert(0, entry.clone());
        } else {
            reordered.push(entry.clone());
        }
    }
    reordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_entries_move_to_the_front_in_reverse_order() {
        let order = [
            "HD.EmbRAID.1-1",
            "NIC.Slot.1-1",
            "Generic.USB.1-1",
            "NIC.Slot.2-1",
        ]
        .map(str::to_string);
        assert_eq!(
            category_first(&order, "nic."),
            [
                "NIC.Slot.2-1",
                "NIC.Slot.1-1",
                "HD.EmbRAID.1-1",
                "Generic.USB.1-1"
            ]
        );
    }
}
