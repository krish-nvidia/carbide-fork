/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::ResetType;
use serde_json::json;

use crate::bmc_control::standard;
use crate::dell;

/// Dell iDRAC manager-control behavior.
///
/// iDRAC configures NTP and time zone through its OEM manager attributes; the
/// three NTP slots are always written so stale servers are cleared.
pub(crate) struct IdracBmcControl;

#[async_trait]
impl<B> BmcControl<B> for IdracBmcControl
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        standard::manager_reset(cx, ResetType::GracefulRestart).await
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::reset_to_factory_defaults(cx, &standard::FactoryDefaults::Standard).await
    }

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        let slot = |index: usize| servers.get(index).map_or("", String::as_str);
        dell::patch_manager_attributes(
            cx,
            json!({
                "NTPConfigGroup.1.NTPEnable": if servers.is_empty() { "Disabled" } else { "Enabled" },
                "NTPConfigGroup.1.NTP1": slot(0),
                "NTPConfigGroup.1.NTP2": slot(1),
                "NTPConfigGroup.1.NTP3": slot(2),
            }),
        )
        .await
    }

    async fn set_utc_timezone(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        dell::patch_manager_attributes(cx, json!({"Time.1.Timezone": "UTC"})).await
    }

    async fn ipmi_over_lan_enabled(&self, cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        standard::ipmi_over_lan_enabled(cx).await
    }

    async fn set_ipmi_over_lan(
        &self,
        cx: &OpCx<'_, B>,
        enabled: bool,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::update_network_protocol(cx, &standard::ipmi_payload(enabled)).await
    }
}
