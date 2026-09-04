/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Fetched, OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc, EntityTypeRef};
use nv_redfish::resource::{PowerState, ResetType};
use serde::Deserialize;

use crate::power::standard::{self, power_state_from_supplies, power_supplies};

/// Lite-On power-shelf power behavior.
///
/// The shelf's ComputerSystem accepts standard resets, but its power state
/// is only meaningful as the aggregate of the supplies' OEM `PowerState`.
pub(crate) struct LiteOnPowerShelfPower;

/// Lite-On reports each supply's state as a top-level boolean `PowerState`.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SupplyPowerState {
    power_state: Option<bool>,
}

#[async_trait]
impl<B> Power<B> for LiteOnPowerShelfPower
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn state(&self, cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
        let mut states = Vec::new();
        for supply in power_supplies(cx).await? {
            let supply = cx
                .bmc()
                .get::<Fetched<SupplyPowerState>>(supply.raw().odata_id())
                .await
                .map_err(|error| cx.map_bmc_error(error))?;
            states.push(supply.power_state);
        }
        power_state_from_supplies(&states)
    }

    async fn ac_power_cycle_supported(&self, _cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        Ok(false)
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::reset(cx, reset_type).await
    }

    async fn chassis_reset(
        &self,
        cx: &OpCx<'_, B>,
        chassis_id: &str,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::chassis_reset(cx, chassis_id, reset_type).await
    }
}
