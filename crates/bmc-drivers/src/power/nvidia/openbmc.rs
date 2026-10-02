/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::oem::nvidia::AuxPowerResetType;
use nv_redfish::resource::ResetType;

use crate::power::standard::{self, StandardPower};

/// NVIDIA OpenBMC platforms cycle auxiliary power through the BMC chassis.
pub(crate) struct OpenBmcPower;

/// The chassis whose OEM action cycles auxiliary power.
const BMC_CHASSIS_ID: &str = "BMC_0";

/// Cycles auxiliary power through the BMC chassis OEM action.
async fn aux_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    standard::chassis(cx, BMC_CHASSIS_ID)
        .await?
        .oem_nvidia_actions()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .aux_power_reset(AuxPowerResetType::AuxPowerCycle)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

#[async_trait]
impl<B: Bmc> Power<B> for OpenBmcPower
where
    B::Error: ActionError,
{
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
