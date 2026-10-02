/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Power mechanics shared by several vendor drivers.

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::chassis::PowerSupply;
use nv_redfish::core::Bmc;
use nv_redfish::resource::PowerState;

/// Restarts the host over IPMI when the runtime attached it, for hosts whose
/// Redfish restart cuts power to their DPUs.
pub(super) async fn ipmi_restart<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    cx.ipmi()
        .ok_or(PlatformError::Unsupported)?
        .chassis_power_reset()
        .await
        .map(|()| DriverOutcome::complete())
}

/// Aggregates per-supply states into one shelf state; `None` when there are no
/// supplies or they disagree or do not report.
pub(super) fn power_state_from_supplies(states: &[Option<bool>]) -> Option<PowerState> {
    if states.is_empty() {
        None
    } else if states.iter().all(|state| *state == Some(true)) {
        Some(PowerState::On)
    } else if states.iter().all(|state| *state == Some(false)) {
        Some(PowerState::Off)
    } else {
        None
    }
}

/// The power supplies of the first chassis that has any.
pub(super) async fn power_supplies<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Vec<PowerSupply<B>>, PlatformError> {
    let chassis = cx
        .service_root()
        .chassis()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?;
    for chassis in &chassis {
        let supplies = chassis
            .power_supplies()
            .await
            .map_err(|error| cx.map_redfish_error(error))?;
        if !supplies.is_empty() {
            return Ok(supplies);
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use carbide_test_support::value_scenarios;

    use super::*;

    #[test]
    fn supplies_agree_on_a_state_or_report_it_unknown() {
        value_scenarios!(run = |states: Vec<Option<bool>>| power_state_from_supplies(&states);
            "agreeing supplies" {
                vec![Some(true), Some(true)] => Some(PowerState::On),
                vec![Some(false)] => Some(PowerState::Off),
            }
            "unknown" {
                Vec::new() => None,
                vec![Some(true), Some(false)] => None,
                vec![Some(true), None] => None,
            }
        );
    }
}
