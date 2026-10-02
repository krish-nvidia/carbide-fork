/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::bmc_control::standard::StandardBmcControl;
use crate::dell;

/// Dell iDRAC manager-control behavior.
///
/// iDRAC configures NTP and time zone through its OEM manager attributes; the
/// three NTP slots are always written so stale servers are cleared, and an
/// empty list leaves NTP unchanged.
pub(crate) struct IdracBmcControl;

#[async_trait]
impl<B: Bmc> BmcControl<B> for IdracBmcControl
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        if servers.is_empty() {
            return Ok(DriverOutcome::complete());
        }
        let slot = |index: usize| servers.get(index).map_or("", String::as_str);
        dell::patch_manager_attributes(
            cx,
            json!({
                "NTPConfigGroup.1.NTPEnable": "Enabled",
                "NTPConfigGroup.1.NTP1": slot(0),
                "NTPConfigGroup.1.NTP2": slot(1),
                "NTPConfigGroup.1.NTP3": slot(2),
            }),
            None,
        )
        .await
    }

    async fn set_utc_timezone(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        dell::patch_manager_attributes(cx, json!({"Time.1.Timezone": "UTC"}), None).await
    }
}
