/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::num::NonZeroU64;

use async_trait::async_trait;
use bmc_platform::{ControllerAction, DriverOutcome, OpCx, PlatformError, Power, Quirk};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::{PowerState, ResetType};

use crate::power::lenovo::support::ac_power_cycle;
use crate::power::standard::{self, StandardPower};

/// Lenovo XClarity Controller power behavior.
///
/// With [`Quirk::LenovoForceRestartHangs`], a ForceRestart powers the host off
/// and, after a wait, back on instead.
pub(crate) struct XccPower;

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
impl<B: Bmc> Power<B> for XccPower
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
            ResetType::ForceRestart if cx.has_quirk(Quirk::LenovoForceRestartHangs) => {
                Ok(force_restart_outcome(standard::state(cx).await?))
            }
            ResetType::FullPowerCycle => ac_power_cycle(cx).await,
            other => self.standard().set(cx, other).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hanging_force_restart_powers_off_before_delayed_power_on() {
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
