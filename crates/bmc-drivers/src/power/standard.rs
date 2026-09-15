/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish power control.

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::Resource;
use nv_redfish::chassis::Chassis;
use nv_redfish::core::Bmc;
use nv_redfish::resource::{PowerState, ResetType};
use nv_redfish::schema::chassis::ChassisResetAction;
use nv_redfish::schema::computer_system::ComputerSystemResetAction;

/// Spec-compliant Redfish power control.
pub(crate) struct StandardPower;

#[async_trait]
impl<B: Bmc> Power<B> for StandardPower {
    fn standard(&self) -> &dyn Power<B> {
        self
    }

    async fn state(&self, cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
        state(cx)
    }

    /// Standard Redfish offers no AC power cycle.
    async fn ac_power_cycle_supported(&self, _cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        Ok(false)
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        reset(cx, reset_type).await
    }

    async fn chassis_reset(
        &self,
        cx: &OpCx<'_, B>,
        chassis_id: &str,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        chassis_reset(cx, chassis_id, reset_type).await
    }
}

/// The selected system's reported power state.
pub(super) fn state<B: Bmc>(cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
    cx.system()?.power_state().ok_or(PlatformError::NoContent)
}

/// Resets the selected system through its advertised `ComputerSystem.Reset` action.
async fn reset<B: Bmc>(
    cx: &OpCx<'_, B>,
    reset_type: ResetType,
) -> Result<DriverOutcome, PlatformError> {
    let system = cx.system()?.raw();
    let action = system
        .actions
        .as_ref()
        .and_then(|actions| actions.reset.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    already_satisfied_is_complete(
        cx.action(
            action,
            &ComputerSystemResetAction {
                reset_type: Some(reset_type),
            },
        )
        .await,
    )
}

/// Resets the chassis with id `chassis_id` through its advertised `Chassis.Reset` action.
async fn chassis_reset<B: Bmc>(
    cx: &OpCx<'_, B>,
    chassis_id: &str,
    reset_type: ResetType,
) -> Result<DriverOutcome, PlatformError> {
    let chassis = chassis(cx, chassis_id).await?.raw();
    let action = chassis
        .actions
        .as_ref()
        .and_then(|actions| actions.reset.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    already_satisfied_is_complete(
        cx.action(
            action,
            &ChassisResetAction {
                reset_type: Some(reset_type),
            },
        )
        .await,
    )
}

/// The chassis with id `chassis_id`; `Unsupported` when the BMC lists none.
pub(super) async fn chassis<B: Bmc>(
    cx: &OpCx<'_, B>,
    chassis_id: &str,
) -> Result<Chassis<B>, PlatformError> {
    cx.service_root()
        .chassis()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .find(|chassis| chassis.id().into_inner() == chassis_id)
        .ok_or(PlatformError::Unsupported)
}

/// BMCs answer a reset that would not change the power state with 409, which
/// for power control means the requested state already holds.
fn already_satisfied_is_complete(
    result: Result<DriverOutcome, PlatformError>,
) -> Result<DriverOutcome, PlatformError> {
    match result {
        Err(PlatformError::Bmc { status: 409, .. }) => Ok(DriverOutcome::complete()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use bmc_platform::{EtagMode, PlatformIdentity, SystemIdentity};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn standard_reset_posts_to_the_selected_systems_advertised_action() {
        let bmc = bmc_mock::test_support::dell_poweredge_r750_bmc().await;
        let identity = PlatformIdentity {
            system: Some(SystemIdentity {
                id: "System.Embedded.1".to_string(),
                ..SystemIdentity::default()
            }),
            ..PlatformIdentity::default()
        };
        let cx = OpCx::new(
            bmc.bmc.as_ref(),
            bmc.service_root.as_ref(),
            &identity,
            EtagMode::default(),
        )
        .await
        .expect("selected system resolves");
        bmc.http_client.take_requests();

        let outcome = StandardPower
            .set(&cx, ResetType::ForceOff)
            .await
            .expect("reset succeeds");
        assert_eq!(outcome, DriverOutcome::complete());

        let requests = bmc.http_client.take_requests();
        let request = requests
            .iter()
            .find(|request| request.method == "POST")
            .expect("a POST was issued");
        assert!(
            request
                .uri
                .ends_with("/redfish/v1/Systems/System.Embedded.1/Actions/ComputerSystem.Reset"),
            "{}",
            request.uri
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request.body).expect("JSON body"),
            json!({"ResetType": "ForceOff"})
        );
    }
}
