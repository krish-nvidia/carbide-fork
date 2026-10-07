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

use super::{boots, http_option_name};
use crate::boot_order::nvidia::{settings_override, write_settings_boot_order};
use crate::boot_order::standard::StandardBootOrder;
use crate::boot_order::support::{
    RedfishBootOrderExt as _, boot_order, display_name, listed_options, persistent_device,
};

/// NVIDIA GB200 and GB300 tray boot behavior.
///
/// Overrides and the boot order go through the system's `Settings` object.
/// The DPU's HTTP option is matched by the start of its name, which some
/// firmware extends.
pub(crate) struct Gbx00BootOrder;

/// Writes the boot order with `option` moved to the front.
async fn option_first<B: Bmc>(
    cx: &OpCx<'_, B>,
    option: &BootOption<B>,
) -> Result<DriverOutcome, PlatformError> {
    let id = option.raw().id.clone();
    let mut order = boot_order(cx.system().await?);
    order.retain(|entry| *entry != id);
    order.insert(0, id);
    write_settings_boot_order(cx, order).await
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for Gbx00BootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        let name = http_option_name(&cx.boot_interface_mac(selector).await?);
        let order = boot_order(cx.system().await?);
        let options = cx.boot_options().await?;
        let listed = listed_options(&order, &options)?;
        let expected = listed
            .iter()
            .map(|option| display_name(option))
            .find(|display_name| display_name.starts_with(&name));
        Ok(BootOrderStatus {
            boot_interface_first: expected
                .zip(listed.first())
                .is_some_and(|(expected, first)| display_name(first) == expected),
            disk_enabled: true,
            other_network_options_disabled: true,
        })
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        let Some(device) = persistent_device(override_setting) else {
            return settings_override(cx, override_setting, false).await;
        };
        let options = cx.boot_options().await?;
        let option = options
            .iter()
            .find(|option| boots(option, device))
            .ok_or_else(|| PlatformError::MissingBootOption {
                description: format!("{device:?} boot option"),
            })?;
        option_first(cx, option).await
    }

    async fn configure(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        let name = http_option_name(&cx.boot_interface_mac(selector).await?);
        let options = cx.boot_options().await?;
        let option = options
            .iter()
            .find(|option| display_name(option).starts_with(&name))
            .ok_or(PlatformError::MissingBootOption { description: name })?;
        option_first(cx, option).await
    }
}
