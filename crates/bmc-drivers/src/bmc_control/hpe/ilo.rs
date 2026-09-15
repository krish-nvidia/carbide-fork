/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId};
use serde_json::json;

use crate::bmc_control::standard::StandardBmcControl;
use crate::bmc_control::support::manager_oem_action;

/// HPE iLO manager-control behavior.
///
/// iLO resets to defaults through its OEM action and takes at most two static
/// NTP servers on the manager's `DateTime` resource rather than NetworkProtocol.
pub(crate) struct IloBmcControl;

const MAX_NTP_SERVERS: usize = 2;

#[async_trait]
impl<B: Bmc> BmcControl<B> for IloBmcControl {
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        manager_oem_action(
            cx,
            "Hpe/HpeiLO.ResetToFactoryDefaults",
            &json!({"Action": "HpeiLO.ResetToFactoryDefaults", "ResetType": "Default"}),
        )
        .await
    }

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        let id = ODataId::from(format!("{}/DateTime", cx.manager()?.odata_id()));
        let servers = &servers[..servers.len().min(MAX_NTP_SERVERS)];
        cx.patch_id(&id, None, &json!({"StaticNTPServers": servers}))
            .await
            .map(DriverOutcome::from)
    }
}
