/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Power mechanics shared by several vendor drivers.

use std::time::Duration;

use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::chassis::PowerSupply;
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::{PowerState, ResetType};
use nv_redfish::schema::computer_system::ComputerSystem;
use tokio::time::Instant;

use crate::power::standard::StandardPower;

/// How often the power state is re-read while waiting for a forced power-off.
const POWER_OFF_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long a forced power-off may take to show in the reported power state.
const POWER_OFF_TIMEOUT: Duration = Duration::from_secs(120);

/// Forces the host off and returns once the BMC reports it off, for
/// operations a platform accepts only on a powered-off host.
pub(super) async fn force_off_and_wait<B: Bmc>(cx: &OpCx<'_, B>) -> Result<(), PlatformError>
where
    B::Error: ActionError,
{
    if current_power_state(cx).await? == Some(PowerState::Off) {
        return Ok(());
    }
    StandardPower.set(cx, ResetType::ForceOff).await?;
    let deadline = Instant::now() + POWER_OFF_TIMEOUT;
    loop {
        tokio::time::sleep(POWER_OFF_POLL_INTERVAL).await;
        let state = current_power_state(cx).await?;
        if state == Some(PowerState::Off) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(PlatformError::Timeout {
                message: format!(
                    "host still reports power state {state:?} {}s after ForceOff",
                    POWER_OFF_TIMEOUT.as_secs()
                ),
            });
        }
    }
}

/// The selected system's power state as the BMC reports it now; the
/// context's resolved system keeps the state it was fetched with.
async fn current_power_state<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<PowerState>, PlatformError> {
    let uri = cx.system().await?.raw().odata_id.clone();
    let system = cx
        .bmc()
        .get::<ComputerSystem>(&uri)
        .await
        .map_err(|error| cx.map_bmc_error(error))?;
    Ok(system.power_state.flatten())
}

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
    use serde_json::json;

    use super::*;
    use crate::test_support::{Fixture, body, path};

    #[tokio::test(start_paused = true)]
    async fn a_host_that_never_reports_off_times_out_after_one_force_off() {
        const SYSTEM: &str = "/redfish/v1/Systems/1";
        const RESET: &str = "/redfish/v1/Systems/1/Actions/ComputerSystem.Reset";
        let bmc = Fixture::new("Contoso", "Server", "1", "1")
            .document(
                SYSTEM,
                json!({
                    "@odata.id": SYSTEM,
                    "Id": "1",
                    "Name": "System",
                    "PowerState": "On",
                    "Actions": {"#ComputerSystem.Reset": {"target": RESET}},
                }),
            )
            .build()
            .await;
        let cx = bmc.cx().await;

        assert!(matches!(
            force_off_and_wait(&cx).await,
            Err(PlatformError::Timeout { .. })
        ));
        assert_eq!(
            bmc.writes()
                .iter()
                .map(|write| (path(write).to_string(), body(write)))
                .collect::<Vec<_>>(),
            [(RESET.to_string(), json!({"ResetType": "ForceOff"}))]
        );
    }

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
