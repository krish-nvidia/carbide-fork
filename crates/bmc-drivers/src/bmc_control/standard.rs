/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish manager control.

use async_trait::async_trait;
use bmc_platform::{
    BmcControl, DriverOutcome, ManagerSettings, ManagerSettingsStatus, OpCx, PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::manager::{ManagerNetworkProtocolUpdate, ManagerResetToDefaultsType};
use nv_redfish::resource::ResetType;
use nv_redfish::schema::manager_network_protocol::{NtpProtocolUpdate, ProtocolUpdate};

use crate::bmc_control::support::RedfishBmcControlExt as _;

/// DMTF manager control.
pub(crate) struct StandardBmcControl;

#[async_trait]
impl<B: Bmc> BmcControl<B> for StandardBmcControl
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn BmcControl<B> {
        self
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        cx.reset_manager(ResetType::GracefulRestart).await
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        cx.manager()
            .await?
            .reset_to_defaults(ManagerResetToDefaultsType::ResetAll)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
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
        cx.update_manager_network_protocol(
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

    /// Redfish has no standard time-zone setting, so there is nothing to change.
    async fn set_utc_timezone(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }

    async fn ipmi_over_lan_enabled(&self, cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        cx.manager()
            .await?
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
        cx.update_manager_network_protocol(
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

    /// Redfish has no standard manager attributes, so only an empty profile applies.
    async fn apply_settings(
        &self,
        _cx: &OpCx<'_, B>,
        profile: &ManagerSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        if profile.attributes.is_empty() {
            Ok(DriverOutcome::complete())
        } else {
            Err(PlatformError::Unsupported)
        }
    }

    async fn settings_status(
        &self,
        _cx: &OpCx<'_, B>,
    ) -> Result<ManagerSettingsStatus, PlatformError> {
        Ok(ManagerSettingsStatus {
            is_applied: true,
            differences: Vec::new(),
        })
    }
}
