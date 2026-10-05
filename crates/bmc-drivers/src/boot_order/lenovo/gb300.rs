/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::computer_system::{BootOption, BootOptionUpdate};
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::schema::computer_system::{BootSource, BootUpdate};
use serde_json::Value;

use crate::boot_order::ami::megarac;
use crate::boot_order::standard::StandardBootOrder;
use crate::boot_order::support::{
    alias, boot_interface_mac, boot_options, boot_order, is_first, reference,
};

/// Lenovo GB300 boot behavior: AMI's, and the GB300 firmware also boots
/// through any other enabled network option, so boot order setup enables only
/// the selected interface's HTTP option among them.
pub(crate) struct Gb300BootOrder;

/// The boot options whose enablement boot order setup decides, each with the
/// enablement it wants: the HTTP option enabled, every PXE or HTTP option
/// besides it disabled.
fn wanted_enablement<'a, B: Bmc>(
    options: &'a [BootOption<B>],
    target: &'a BootOption<B>,
) -> impl Iterator<Item = (&'a BootOption<B>, bool)> {
    let target = reference(target);
    options
        .iter()
        .filter(move |option| {
            reference(option) == target
                || matches!(alias(option), Some(BootSource::Pxe | BootSource::UefiHttp))
        })
        .map(move |option| (option, reference(option) == target))
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for Gb300BootOrder {
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
        let target = megarac::http_option(&options, &mac);
        Ok(BootOrderStatus {
            boot_interface_first: target.is_some_and(|target| is_first(&order, reference(target))),
            disk_enabled: true,
            other_network_options_disabled: target.is_none_or(|target| {
                wanted_enablement(&options, target)
                    .all(|(option, enabled)| option.enabled() == Some(enabled))
            }),
        })
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        megarac::set_override(cx, override_setting).await
    }

    /// Writes each option's enablement to its `SD` settings object with
    /// `If-Match: *`.
    async fn configure(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        let mac = boot_interface_mac(cx, selector).await?;
        let options = boot_options(cx).await?;
        let target = megarac::http_option(&options, &mac)
            .ok_or_else(|| megarac::missing_http_option(&mac))?;
        let mut outcome = megarac::put_first(cx, boot_order(cx.system().await?), target).await?;
        for (option, enabled) in wanted_enablement(&options, target) {
            if option.enabled() == Some(enabled) {
                continue;
            }
            let settings = ODataId::from(format!("{}/SD", option.raw().odata_id));
            let body = BootOptionUpdate::builder()
                .with_boot_option_enabled(enabled)
                .build();
            let response = cx
                .bmc()
                .update::<_, Value>(&settings, None, &body)
                .await
                .map_err(|error| cx.map_bmc_error(error))?;
            outcome = outcome.merge(DriverOutcome::from(response));
        }
        Ok(outcome)
    }
}
