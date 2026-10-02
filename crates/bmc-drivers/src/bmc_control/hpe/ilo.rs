/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::oem::hpe::HpeManager;
use nv_redfish::oem::hpe::date_time::HpeiLoDateTimeUpdate;

use crate::bmc_control::standard::StandardBmcControl;

/// HPE iLO manager-control behavior.
///
/// iLO resets to defaults through its OEM action and takes exactly two static
/// NTP servers on its `DateTime` service rather than NetworkProtocol.
pub(crate) struct IloBmcControl;

fn hpe_manager<B: Bmc>(cx: &OpCx<'_, B>) -> Result<HpeManager<B>, PlatformError> {
    cx.manager()?
        .oem_hpe()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

#[async_trait]
impl<B: Bmc> BmcControl<B> for IloBmcControl
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        hpe_manager(cx)?
            .reset_to_factory_defaults()
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }

    /// An empty list leaves the NTP configuration unchanged; the first two
    /// servers are written, padding a single server with an empty entry.
    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        if servers.is_empty() {
            return Ok(DriverOutcome::complete());
        }
        let static_ntp_servers = vec![
            servers.first().cloned().unwrap_or_default(),
            servers.get(1).cloned().unwrap_or_default(),
        ];
        hpe_manager(cx)?
            .date_time()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .update(
                &HpeiLoDateTimeUpdate::builder()
                    .with_static_ntp_servers(static_ntp_servers)
                    .build(),
            )
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }
}
