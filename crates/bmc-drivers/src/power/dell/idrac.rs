/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::Bmc;
use nv_redfish::resource::{PowerState, ResetType};
use serde_json::json;

use crate::dell;
use crate::power::standard::{self, StandardPower};

/// Dell iDRAC host-power behavior.
///
/// iDRAC has no AC-cycle action; the BIOS `PowerCycleRequest` job runs during
/// the next reset, so the follow-up reset is what actually performs the cycle.
pub(crate) struct IdracPower;

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
impl<B: Bmc> Power<B> for IdracPower {
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
            ResetType::FullPowerCycle => full_power_cycle(cx).await,
            other => self.standard().set(cx, other).await,
        }
    }
}
