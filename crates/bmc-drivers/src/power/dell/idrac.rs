/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::{PowerState, ResetType};
use serde_json::json;

use crate::dell;
use crate::power::standard;

/// Dell iDRAC host-power behavior.
pub(crate) struct IdracPower;

/// iDRAC has no AC-cycle action; the BIOS `PowerCycleRequest` job runs during
/// the next reset, so the follow-up reset is what actually performs the cycle.
async fn full_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let staged =
        dell::stage_bios_attributes(cx, json!({"PowerCycleRequest": "FullPowerCycle"})).await?;
    let follow_up = match standard::state(cx)? {
        PowerState::Off => ResetType::On,
        _ => ResetType::GracefulRestart,
    };
    Ok(staged.then([ControllerAction::Power(follow_up)]))
}

#[async_trait]
impl<B> Power<B> for IdracPower
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn state(&self, cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
        standard::state(cx)
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
            ResetType::FullPowerCycle => full_power_cycle(cx).await,
            other => standard::reset(cx, other).await,
        }
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
