/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::computer_system::BootOption;
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::BootUpdate;

use crate::boot_order::standard::StandardBootOrder;
use crate::boot_order::support::{
    self, alias, boot_interface_mac, boot_options, boot_order, display_name, is_first,
    persistent_device, reference, system_uri, write_override,
};

/// AMI MegaRAC boot behavior.
///
/// The override goes to the live system and the boot order to its `SD`
/// settings object, both with `If-Match: *`; an override without a mode boots
/// UEFI. A persistent device boots first by its boot option's `Alias`.
pub(crate) struct MegaRacBootOrder;

/// The HTTP IPv4 boot option of the interface with `mac`, named after the MAC
/// as in `[Slot2]UEFI: HTTP IPv4 Nvidia Network Adapter - B8:E9:24:17:6D:72 P1`.
pub(crate) fn http_option<'a, B: Bmc>(
    options: &'a [BootOption<B>],
    mac: &str,
) -> Option<&'a BootOption<B>> {
    let mac = mac.to_uppercase();
    options.iter().find(|option| {
        let name = display_name(option).to_uppercase();
        name.contains("HTTP") && name.contains("IPV4") && name.contains(&mac)
    })
}

pub(crate) fn missing_http_option(mac: &str) -> PlatformError {
    PlatformError::MissingBootOption {
        description: format!("HTTP IPv4 boot option for MAC {mac}"),
    }
}

/// Moves `target` first in the `SD` boot order unless `order` already starts
/// with it.
pub(crate) async fn put_first<B: Bmc>(
    cx: &OpCx<'_, B>,
    order: Vec<String>,
    target: &BootOption<B>,
) -> Result<DriverOutcome, PlatformError> {
    let sd = system_uri(cx, Some("SD")).await?;
    support::put_first(cx, &sd, order, reference(target)).await
}

/// Boots the device persistently by moving the boot option whose `Alias`
/// names it first in the `SD` boot order; otherwise sets the override.
pub(crate) async fn set_override<B: Bmc>(
    cx: &OpCx<'_, B>,
    setting: &BootUpdate,
) -> Result<DriverOutcome, PlatformError> {
    let Some(device) = persistent_device(setting) else {
        let system = system_uri(cx, None).await?;
        return write_override(cx, &system, setting, true, None).await;
    };
    let options = boot_options(cx).await?;
    let target = options
        .iter()
        .find(|option| alias(option) == Some(device))
        .ok_or_else(|| PlatformError::MissingBootOption {
            description: format!("boot option with alias {device:?}"),
        })?;
    put_first(cx, boot_order(cx.system().await?), target).await
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for MegaRacBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        let mac = boot_interface_mac(cx, selector).await?;
        let order = boot_order(cx.system().await?);
        let options = boot_options(cx).await?;
        Ok(BootOrderStatus {
            boot_interface_first: http_option(&options, &mac)
                .is_some_and(|target| is_first(&order, reference(target))),
            disk_enabled: true,
            other_network_options_disabled: true,
        })
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
        let mac = boot_interface_mac(cx, selector).await?;
        let options = boot_options(cx).await?;
        let target = http_option(&options, &mac).ok_or_else(|| missing_http_option(&mac))?;
        put_first(cx, boot_order(cx.system().await?), target).await
    }
}
