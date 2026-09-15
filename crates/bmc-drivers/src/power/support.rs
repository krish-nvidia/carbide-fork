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

/// Aggregates per-supply states into one host state; mixed states are an error.
pub(super) fn power_state_from_supplies(
    states: &[Option<bool>],
) -> Result<PowerState, PlatformError> {
    if states.is_empty() {
        return Err(PlatformError::NoContent);
    }
    if states.iter().all(|state| *state == Some(true)) {
        Ok(PowerState::On)
    } else if states.iter().all(|state| *state == Some(false)) {
        Ok(PowerState::Off)
    } else {
        Err(PlatformError::InvalidResponse {
            message: format!("power supplies report mixed or missing states: {states:?}"),
        })
    }
}

/// Every power supply of every chassis.
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
    let mut supplies = Vec::new();
    for chassis in &chassis {
        supplies.extend(
            chassis
                .power_supplies()
                .await
                .map_err(|error| cx.map_redfish_error(error))?,
        );
    }
    Ok(supplies)
}
