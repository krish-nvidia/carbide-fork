/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::Bmc;
use nv_redfish::oem::hpe::HpeSystemResetType;
use nv_redfish::oem::hpe::schema::ActionAnnotations;
use nv_redfish::oem::hpe::schema::hpe_computer_system_ext::HpeComputerSystemExtSystemResetAction;
use nv_redfish::resource::{PowerState, ResetType};

use crate::power::standard::{self, StandardPower};

/// HPE iLO power behavior.
///
/// iLO maps `ForceRestart` to a graceful restart, and its auxiliary power
/// cycle is only accepted while the host is off.
pub(crate) struct IloPower;

/// Posts the iLO auxiliary power cycle, which the BMC accepts only while the host is off.
async fn aux_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let actions = cx
        .system()?
        .oem_hpe_actions()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    let action = actions
        .raw()
        .and_then(|actions| actions.system_reset.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    cx.action(
        action,
        &HpeComputerSystemExtSystemResetAction {
            redfish_annotations: ActionAnnotations::default(),
            reset_type: HpeSystemResetType::AuxCycle,
        },
    )
    .await
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
