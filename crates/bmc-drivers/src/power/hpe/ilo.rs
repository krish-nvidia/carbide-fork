/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::resource::{PowerState, ResetType};
use serde_json::json;

use crate::power::standard::{self, StandardPower};

/// HPE iLO power behavior.
///
/// iLO maps `ForceRestart` to a graceful restart, and its auxiliary power
/// cycle is only accepted while the host is off.
pub(crate) struct IloPower;

/// Posts the iLO auxiliary power cycle, which the BMC accepts only while the host is off.
async fn aux_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let target = ODataId::from(format!(
        "{}/Actions/Oem/Hpe/HpeComputerSystemExt.SystemReset",
        cx.system()?.odata_id()
    ));
    cx.post(&target, &json!({"ResetType": "AuxCycle"})).await
}

#[async_trait]
impl<B: Bmc> Power<B> for IloPower {
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
            ResetType::ForceRestart => self.standard().set(cx, ResetType::GracefulRestart).await,
            ResetType::FullPowerCycle => {
                if standard::state(cx)? != PowerState::Off {
                    return Ok(DriverOutcome::blocked(ControllerAction::Power(
                        ResetType::ForceOff,
                    )));
                }
                aux_power_cycle(cx).await
            }
            other => self.standard().set(cx, other).await,
        }
    }
}
