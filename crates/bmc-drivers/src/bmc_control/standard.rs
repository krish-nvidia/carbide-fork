/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::Resource;
use nv_redfish::core::{ActionError, Bmc, ODataId};
use nv_redfish::manager::ManagerResetToDefaultsType;
use nv_redfish::resource::ResetType;
use serde::Serialize;
use serde_json::{Value, json};

/// How a family restores manager defaults.
pub(crate) enum FactoryDefaults {
    /// The standard `Manager.ResetToDefaults` action.
    Standard,
    /// An OEM action below the manager's `Actions/Oem/`.
    Oem {
        action: &'static str,
        payload: fn() -> Value,
    },
}

/// DMTF manager control; families vary in the reset type their manager
/// accepts, how many NTP servers they take, and how defaults are restored.
pub(crate) struct StandardBmcControl {
    pub(crate) reset_type: ResetType,
    pub(crate) max_ntp_servers: Option<usize>,
    pub(crate) factory_defaults: FactoryDefaults,
}

pub(crate) static STANDARD_BMC_CONTROL: StandardBmcControl = StandardBmcControl {
    reset_type: ResetType::GracefulRestart,
    max_ntp_servers: None,
    factory_defaults: FactoryDefaults::Standard,
};

/// Resets the manager over Redfish, falling back to an IPMI cold reset when
/// the runtime attached IPMI and Redfish refused or could not be reached.
pub(super) async fn manager_reset<B: Bmc>(
    cx: &OpCx<'_, B>,
    reset_type: ResetType,
) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    let redfish = cx
        .manager()?
        .reset(Some(reset_type))
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error));
    match (redfish, cx.ipmi()) {
        (Err(_), Some(ipmi)) => ipmi
            .bmc_cold_reset()
            .await
            .map(|()| DriverOutcome::complete()),
        (result, _) => result,
    }
}

pub(super) async fn reset_to_factory_defaults<B: Bmc>(
    cx: &OpCx<'_, B>,
    method: &FactoryDefaults,
) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    let manager = cx.manager()?;
    match method {
        FactoryDefaults::Standard => manager
            .reset_to_defaults(ManagerResetToDefaultsType::ResetAll)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error)),
        FactoryDefaults::Oem { action, payload } => {
            let target = ODataId::from(format!("{}/Actions/Oem/{action}", manager.odata_id()));
            cx.post(&target, &payload()).await
        }
    }
}

/// Writes to the manager's advertised `NetworkProtocol` resource.
pub(super) async fn update_network_protocol<B, T>(
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

pub(super) fn ntp_payload(servers: &[String]) -> Value {
    json!({"NTP": {"ProtocolEnabled": !servers.is_empty(), "NTPServers": servers}})
}

pub(super) fn ipmi_payload(enabled: bool) -> Value {
    json!({"IPMI": {"ProtocolEnabled": enabled}})
}

pub(super) async fn ipmi_over_lan_enabled<B: Bmc>(cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
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

#[async_trait]
impl<B> BmcControl<B> for StandardBmcControl
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        manager_reset(cx, self.reset_type).await
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        reset_to_factory_defaults(cx, &self.factory_defaults).await
    }

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        let limit = self.max_ntp_servers.unwrap_or(servers.len());
        update_network_protocol(cx, &ntp_payload(&servers[..servers.len().min(limit)])).await
    }

    async fn set_utc_timezone(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    async fn ipmi_over_lan_enabled(&self, cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        ipmi_over_lan_enabled(cx).await
    }

    async fn set_ipmi_over_lan(
        &self,
        cx: &OpCx<'_, B>,
        enabled: bool,
    ) -> Result<DriverOutcome, PlatformError> {
        update_network_protocol(cx, &ipmi_payload(enabled)).await
    }
}
