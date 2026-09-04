/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::{PowerState, ResetType};
use serde_json::json;

use crate::power::standard::{self, Anchor, OemPowerCycle};

/// HPE iLO power behavior.
///
/// iLO maps `ForceRestart` to a graceful restart, and its auxiliary power
/// cycle is only accepted while the host is off.
pub(crate) struct IloPower;

const AUX_CYCLE: OemPowerCycle = OemPowerCycle {
    anchor: Anchor::System,
    action: "Hpe/HpeComputerSystemExt.SystemReset",
    payload: || json!({"ResetType": "AuxCycle"}),
};

#[async_trait]
impl<B> Power<B> for IloPower
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
            ResetType::ForceRestart => standard::reset(cx, ResetType::GracefulRestart).await,
            ResetType::FullPowerCycle => {
                if standard::state(cx)? != PowerState::Off {
                    return Ok(DriverOutcome::blocked(ControllerAction::Power(
                        ResetType::ForceOff,
                    )));
                }
                standard::oem_power_cycle(cx, &AUX_CYCLE).await
            }
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
