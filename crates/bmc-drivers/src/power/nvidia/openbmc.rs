/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::resource::ResetType;
use serde_json::json;

use crate::power::standard::{self, StandardPower};

/// NVIDIA OpenBMC platforms cycle auxiliary power through the BMC chassis.
pub(crate) struct OpenBmcPower;

/// The chassis whose OEM action cycles auxiliary power.
const BMC_CHASSIS_ID: &str = "BMC_0";

/// Cycles auxiliary power through the BMC chassis OEM action.
async fn aux_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let chassis = standard::chassis(cx, BMC_CHASSIS_ID).await?;
    let target = ODataId::from(format!(
        "{}/Actions/Oem/NvidiaChassis.AuxPowerReset",
        chassis.odata_id()
    ));
    cx.post(&target, &json!({"ResetType": "AuxPowerCycle"}))
        .await
}

#[async_trait]
impl<B: Bmc> Power<B> for OpenBmcPower {
    fn standard(&self) -> &dyn Power<B> {
        &StandardPower
    }

    async fn ac_power_cycle_supported(&self, _cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        Ok(true)
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        match reset_type {
            ResetType::FullPowerCycle => aux_power_cycle(cx).await,
            other => self.standard().set(cx, other).await,
        }
    }
}
