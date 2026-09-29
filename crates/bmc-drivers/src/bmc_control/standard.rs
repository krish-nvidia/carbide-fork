/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish manager control.

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::manager::ManagerNetworkProtocolUpdate;
use nv_redfish::resource::ResetType;
use nv_redfish::schema::ActionAnnotations;
use nv_redfish::schema::manager::{
    ManagerResetAction, ManagerResetToDefaultsAction, ResetToDefaultsType,
};
use nv_redfish::schema::manager_network_protocol::{NtpProtocolUpdate, ProtocolUpdate};

use crate::update;

/// DMTF manager control.
pub(crate) struct StandardBmcControl;

#[async_trait]
impl<B: Bmc> BmcControl<B> for StandardBmcControl {
    fn standard(&self) -> &dyn BmcControl<B> {
        self
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        reset(cx, ResetType::GracefulRestart).await
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        reset_to_factory_defaults(cx).await
    }

    /// An empty list leaves the NTP configuration unchanged.
    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        if servers.is_empty() {
            return Ok(DriverOutcome::complete());
        }
        update_network_protocol(
            cx,
            ManagerNetworkProtocolUpdate::builder()
                .with_ntp(
                    NtpProtocolUpdate::builder()
                        .with_ntp_servers(servers.to_vec())
                        .with_protocol_enabled(true)
                        .build(),
                )
                .build(),
        )
        .await
    }

    /// Redfish has no standard time-zone setting.
    async fn set_utc_timezone(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn ipmi_over_lan_enabled(&self, cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        cx.manager()?
            .network_protocol()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .raw()
            .ipmi
            .as_ref()
            .and_then(|ipmi| ipmi.protocol_enabled)
            .flatten()
            .ok_or(PlatformError::NoContent)
    }

    async fn set_ipmi_over_lan(
        &self,
        cx: &OpCx<'_, B>,
        enabled: bool,
    ) -> Result<DriverOutcome, PlatformError> {
        update_network_protocol(
            cx,
            ManagerNetworkProtocolUpdate::builder()
                .with_ipmi(
                    ProtocolUpdate::builder()
                        .with_protocol_enabled(enabled)
                        .build(),
                )
                .build(),
        )
        .await
    }
}

/// Resets the manager over Redfish, falling back to an IPMI cold reset when
/// the runtime attached IPMI and Redfish refused or could not be reached.
pub(super) async fn reset<B: Bmc>(
    cx: &OpCx<'_, B>,
    reset_type: ResetType,
) -> Result<DriverOutcome, PlatformError> {
    let manager = cx.manager()?.raw();
    let redfish = match manager
        .actions
        .as_ref()
        .and_then(|actions| actions.reset.as_ref())
    {
        Some(action) => {
            cx.action(
                action,
                &ManagerResetAction {
                    redfish_annotations: ActionAnnotations::default(),
                    reset_type: Some(reset_type),
                },
            )
            .await
        }
        None => Err(PlatformError::Unsupported),
    };
    match (redfish, cx.ipmi()) {
        (Err(_), Some(ipmi)) => ipmi
            .bmc_cold_reset()
            .await
            .map(|()| DriverOutcome::complete()),
        (result, _) => result,
    }
}

/// Restores every manager setting through the `Manager.ResetToDefaults` action.
async fn reset_to_factory_defaults<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<DriverOutcome, PlatformError> {
    let manager = cx.manager()?.raw();
    let action = manager
        .actions
        .as_ref()
        .and_then(|actions| actions.reset_to_defaults.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    cx.action(
        action,
        &ManagerResetToDefaultsAction {
            redfish_annotations: ActionAnnotations::default(),
            reset_type: ResetToDefaultsType::ResetAll,
        },
    )
    .await
}

/// Writes to the manager's advertised `NetworkProtocol` resource.
async fn update_network_protocol<B: Bmc>(
    cx: &OpCx<'_, B>,
    body: ManagerNetworkProtocolUpdate,
) -> Result<DriverOutcome, PlatformError> {
    let protocol = cx
        .manager()?
        .network_protocol()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    update::apply(cx, protocol.raw().as_ref(), &body, protocol.update(&body)).await
}
