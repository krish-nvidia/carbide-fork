/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BmcControl, DriverOutcome, ManagerSettings, ManagerSettingsDiff, ManagerSettingsStatus, OpCx,
    PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::oem::oem_value;
use nv_redfish::resource::PowerState;
use serde_json::{Value, json};

use crate::bmc_control::standard::StandardBmcControl;
use crate::resources::RedfishResourcesExt as _;

/// NVIDIA Vera Rubin: the host's onboard NICs reach the management network
/// unless the SMM chassis disables their access, which provisioning requires.
pub(crate) struct VeraRubinBmcControl;

/// The chassis that owns host management network access.
const SMM_CHASSIS_ID: &str = "SMM_0";
const HOST_MANAGEMENT_NETWORK_ACCESS: &str = "HostManagementNetworkAccess";

#[async_trait]
impl<B: Bmc> BmcControl<B> for VeraRubinBmcControl
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    /// Disables host management network access. The SMM takes the change
    /// only while the host is on, and the stored value can disagree with the
    /// NICs after an AUX power cycle, so it is written every time.
    async fn apply_settings(
        &self,
        cx: &OpCx<'_, B>,
        profile: &ManagerSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        if !profile.attributes.is_empty() {
            return Err(PlatformError::Unsupported);
        }
        if cx.system().await?.power_state() != Some(PowerState::On) {
            return Err(PlatformError::NotReady {
                message: "host management network access changes only while the host is on"
                    .to_string(),
            });
        }
        let chassis = cx
            .chassis_by_id(SMM_CHASSIS_ID)
            .await?
            .ok_or(PlatformError::Unsupported)?;
        cx.bmc()
            .update::<_, Value>(
                &chassis.raw().odata_id,
                None,
                &json!({"Oem": {"Nvidia": {HOST_MANAGEMENT_NETWORK_ACCESS: false}}}),
            )
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_bmc_error(error))
    }

    /// A missing value counts as a difference: access is applied only when
    /// the SMM reports it disabled.
    async fn settings_status(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<ManagerSettingsStatus, PlatformError> {
        let chassis = cx
            .chassis_by_id(SMM_CHASSIS_ID)
            .await?
            .ok_or(PlatformError::Unsupported)?;
        let actual = chassis
            .raw()
            .oem
            .as_ref()
            .and_then(|oem| oem_value(oem, "Nvidia"))
            .and_then(|nvidia| nvidia.get(HOST_MANAGEMENT_NETWORK_ACCESS))
            .cloned();
        let differences: Vec<ManagerSettingsDiff> = (actual != Some(json!(false)))
            .then(|| ManagerSettingsDiff {
                key: HOST_MANAGEMENT_NETWORK_ACCESS.to_string(),
                expected: json!(false),
                actual,
            })
            .into_iter()
            .collect();
        Ok(ManagerSettingsStatus {
            is_applied: differences.is_empty(),
            differences,
        })
    }
}
