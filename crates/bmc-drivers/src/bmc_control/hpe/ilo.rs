/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::Resource;
use nv_redfish::core::{ActionError, Bmc, ODataId};
use nv_redfish::resource::ResetType;
use serde_json::json;

use crate::bmc_control::standard::{self, FactoryDefaults};

/// HPE iLO manager-control behavior.
///
/// iLO resets to defaults through its OEM action and takes at most two static
/// NTP servers on the manager's `DateTime` resource rather than NetworkProtocol.
pub(crate) struct IloBmcControl;

const FACTORY_DEFAULTS: FactoryDefaults = FactoryDefaults::Oem {
    action: "Hpe/HpeiLO.ResetToFactoryDefaults",
    payload: || json!({"Action": "HpeiLO.ResetToFactoryDefaults", "ResetType": "Default"}),
};

#[async_trait]
impl<B> BmcControl<B> for IloBmcControl
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
        standard::reset_to_factory_defaults(cx, &FACTORY_DEFAULTS).await
    }

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        let id = ODataId::from(format!("{}/DateTime", cx.manager()?.odata_id()));
        let servers = &servers[..servers.len().min(2)];
        cx.patch_id(&id, None, &json!({"StaticNTPServers": servers}))
            .await
            .map(DriverOutcome::from)
    }

    async fn set_utc_timezone(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
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
