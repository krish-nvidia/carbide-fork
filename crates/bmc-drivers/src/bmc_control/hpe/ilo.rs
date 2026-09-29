/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::oem::hpe::HpeManager;
use nv_redfish::oem::hpe::date_time::HpeiLoDateTimeUpdate;
use nv_redfish::oem::hpe::manager::ResetType;
use nv_redfish::oem::hpe::schema::ActionAnnotations;
use nv_redfish::oem::hpe::schema::hpei_lo::HpeiLOResetToFactoryDefaultsAction;

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
impl<B: Bmc> BmcControl<B> for IloBmcControl {
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        let manager = hpe_manager(cx)?.raw();
        let action = manager
            .actions
            .as_ref()
            .and_then(|actions| actions.reset_to_factory_defaults.as_ref())
            .ok_or(PlatformError::Unsupported)?;
        cx.action(
            action,
            &HpeiLOResetToFactoryDefaultsAction {
                redfish_annotations: ActionAnnotations::default(),
                reset_type: ResetType::Default,
            },
        )
        .await
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
