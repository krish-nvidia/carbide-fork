/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Shared standard providers reused across plugins.
//!
//! These wrap the conventional Redfish shapes (power state, `ComputerSystem.
//! Reset`, `Manager.Reset`, manager firmware) on top of the [`RedfishOps`]
//! façade, so vendor plugins delegate here and override only their
//! model/firmware-specific behavior. Nothing here touches the transport
//! directly -- it is all paths and JSON.

use carbide_redfish_platform_api::RedfishError;
use carbide_redfish_platform_api::model::{PowerAction, PowerState};
use carbide_redfish_platform_api::ops::PlatformExecutionContext;
use serde_json::json;

/// The standard `ComputerSystem.Reset` `ResetType` for a power action, or
/// `None` when the action has no standard mapping (`AcPowerCycle`, which is a
/// vendor OEM action).
pub fn reset_type(action: PowerAction) -> Option<&'static str> {
    Some(match action {
        PowerAction::On => "On",
        PowerAction::GracefulShutdown => "GracefulShutdown",
        PowerAction::ForceOff => "ForceOff",
        PowerAction::GracefulRestart => "GracefulRestart",
        PowerAction::ForceRestart => "ForceRestart",
        PowerAction::PowerCycle => "PowerCycle",
        PowerAction::AcPowerCycle => return None,
    })
}

/// Standard `Chassis.Reset` on a chassis member.
pub async fn standard_chassis_reset(
    ctx: &PlatformExecutionContext<'_>,
    chassis_id: &str,
    reset_type: &str,
) -> Result<(), RedfishError> {
    let target = format!("/redfish/v1/Chassis/{chassis_id}/Actions/Chassis.Reset");
    ctx.ops()
        .post_action(&target, json!({ "ResetType": reset_type }))
        .await?;
    Ok(())
}

/// Set the canonical manager's timezone offset to UTC.
pub async fn standard_set_bmc_time_utc(
    ctx: &PlatformExecutionContext<'_>,
) -> Result<(), RedfishError> {
    let manager_path = ctx.ops().manager_path().await?;
    ctx.ops()
        .patch(&manager_path, json!({ "DateTimeLocalOffset": "+00:00" }))
        .await?;
    Ok(())
}

/// Read the canonical manager's firmware version and clock in one GET.
pub async fn standard_manager_status(
    ctx: &PlatformExecutionContext<'_>,
) -> Result<(Option<String>, Option<String>), RedfishError> {
    let manager_path = ctx.ops().manager_path().await?;
    let manager = ctx.ops().get(&manager_path).await?;
    let firmware = manager
        .get("FirmwareVersion")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let date_time = manager
        .get("DateTime")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    Ok((firmware, date_time))
}

/// Read host power state via the canonical system.
pub async fn standard_power_state(
    ctx: &PlatformExecutionContext<'_>,
) -> Result<PowerState, RedfishError> {
    let system_path = ctx.ops().system_path().await?;
    let system = ctx.ops().get(&system_path).await?;
    Ok(match system.get("PowerState").and_then(|v| v.as_str()) {
        Some("On") => PowerState::On,
        Some("Off") => PowerState::Off,
        _ => PowerState::Unknown,
    })
}

/// Standard `ComputerSystem.Reset` power control.
pub async fn standard_set_power(
    ctx: &PlatformExecutionContext<'_>,
    reset_type: &str,
) -> Result<(), RedfishError> {
    let system_path = ctx.ops().system_path().await?;
    let target = format!("{system_path}/Actions/ComputerSystem.Reset");
    ctx.ops()
        .post_action(&target, json!({ "ResetType": reset_type }))
        .await?;
    Ok(())
}

/// Standard `Manager.Reset` BMC reset.
pub async fn standard_reset_bmc(
    ctx: &PlatformExecutionContext<'_>,
    reset_type: &str,
) -> Result<(), RedfishError> {
    let manager_path = ctx.ops().manager_path().await?;
    let target = format!("{manager_path}/Actions/Manager.Reset");
    ctx.ops()
        .post_action(&target, json!({ "ResetType": reset_type }))
        .await?;
    Ok(())
}

/// Read the canonical manager's firmware version, if present.
pub async fn standard_manager_firmware(
    ctx: &PlatformExecutionContext<'_>,
) -> Result<Option<String>, RedfishError> {
    let manager_path = ctx.ops().manager_path().await?;
    let manager = ctx.ops().get(&manager_path).await?;
    Ok(manager
        .get("FirmwareVersion")
        .and_then(|v| v.as_str())
        .map(ToString::to_string))
}
