/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::num::NonZeroU64;

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::{PowerState, ResetType};

use super::xcc::XCC_AC_POWER_CYCLE;
use crate::power::standard;

/// Lenovo ThinkSystem SR675 V3 OVX power workaround.
///
/// The selection rule limits this driver to SKU `7D9RCTOLWW` with UEFI 7.10
/// and BMC 9.10, where a standard ForceRestart can hang.
pub(crate) struct Sr675V3OvxPower;

fn force_restart_outcome(state: PowerState) -> DriverOutcome {
    if state != PowerState::Off {
        DriverOutcome::blocked(ControllerAction::Power(ResetType::ForceOff))
    } else {
        DriverOutcome::Complete {
            follow_up: vec![
                ControllerAction::Wait {
                    seconds: NonZeroU64::new(10).expect("workaround wait is nonzero"),
                },
                ControllerAction::Power(ResetType::On),
            ],
        }
    }
}

#[async_trait]
impl<B> Power<B> for Sr675V3OvxPower
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
            ResetType::ForceRestart => Ok(force_restart_outcome(standard::state(cx)?)),
            ResetType::FullPowerCycle => standard::oem_power_cycle(cx, &XCC_AC_POWER_CYCLE).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workaround_powers_off_before_delayed_power_on() {
        assert_eq!(
            force_restart_outcome(PowerState::On),
            DriverOutcome::blocked(ControllerAction::Power(ResetType::ForceOff))
        );
        assert_eq!(
            force_restart_outcome(PowerState::Off),
            DriverOutcome::Complete {
                follow_up: vec![
                    ControllerAction::Wait {
                        seconds: NonZeroU64::new(10).expect("nonzero"),
                    },
                    ControllerAction::Power(ResetType::On),
                ],
            }
        );
    }
}
