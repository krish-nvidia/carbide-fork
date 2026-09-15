/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish manager control.

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::resource::ResetType;
use nv_redfish::schema::manager::{
    ManagerResetAction, ManagerResetToDefaultsAction, ResetToDefaultsType,
};
use serde::Serialize;
use serde_json::json;

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

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        update_network_protocol(
            cx,
            &json!({"NTP": {"ProtocolEnabled": !servers.is_empty(), "NTPServers": servers}}),
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
        update_network_protocol(cx, &json!({"IPMI": {"ProtocolEnabled": enabled}})).await
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
            reset_type: ResetToDefaultsType::ResetAll,
        },
    )
    .await
}

/// Writes to the manager's advertised `NetworkProtocol` resource.
async fn update_network_protocol<B, T>(
    cx: &OpCx<'_, B>,
    payload: &T,
) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    T: Serialize + Send + Sync,
{
    let protocol = cx
        .manager()?
        .network_protocol()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    cx.patch(protocol.raw().as_ref(), payload).await
}
