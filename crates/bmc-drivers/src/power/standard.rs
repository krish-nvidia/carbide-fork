/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::Resource;
use nv_redfish::chassis::PowerSupply;
use nv_redfish::core::{ActionError, Bmc, ODataId};
use nv_redfish::resource::{PowerState, ResetType};
use serde_json::Value;

/// An OEM reset action that performs the AC (full) power cycle the standard
/// `ComputerSystem.Reset` does not offer.
pub(crate) struct OemPowerCycle {
    /// Which resource carries the action.
    pub(crate) anchor: Anchor,
    /// Action path below the anchor's `Actions/Oem/`.
    pub(crate) action: &'static str,
    /// Request body.
    pub(crate) payload: fn() -> Value,
}

/// Resource an OEM power action hangs off.
pub(crate) enum Anchor {
    /// The selected ComputerSystem.
    System,
    /// The chassis with this id.
    Chassis(&'static str),
}

/// How the driver performs `ForceRestart` and `GracefulRestart`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Restart {
    /// The standard `ComputerSystem.Reset` action.
    Redfish,
    /// `ipmitool chassis power reset`: for hosts whose Redfish restart cuts
    /// power to their DPUs.
    Ipmi,
}

/// Redfish power control; families vary only in how they restart the host
/// and whether an OEM action provides a full power cycle.
pub(crate) struct StandardPower {
    pub(crate) restart: Restart,
    pub(crate) full_power_cycle: Option<OemPowerCycle>,
}

pub(crate) static STANDARD_POWER: StandardPower = StandardPower {
    restart: Restart::Redfish,
    full_power_cycle: None,
};

pub(super) fn state<B: Bmc>(cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
    cx.system()?.power_state().ok_or(PlatformError::NoContent)
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

pub(super) async fn reset<B: Bmc>(
    cx: &OpCx<'_, B>,
    reset_type: ResetType,
) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    already_satisfied_is_complete(
        cx.system()?
            .reset(Some(reset_type))
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error)),
    )
}

/// Restarts the host over IPMI when the runtime attached it.
pub(super) async fn ipmi_restart<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    cx.ipmi()
        .ok_or(PlatformError::Unsupported)?
        .chassis_power_reset()
        .await
        .map(|()| DriverOutcome::complete())
}

/// Posts an OEM power-cycle action.
pub(super) async fn oem_power_cycle<B: Bmc>(
    cx: &OpCx<'_, B>,
    cycle: &OemPowerCycle,
) -> Result<DriverOutcome, PlatformError> {
    let anchor = match cycle.anchor {
        Anchor::System => cx.system()?.odata_id().clone(),
        Anchor::Chassis(chassis_id) => chassis(cx, chassis_id).await?.odata_id().clone(),
    };
    let target = ODataId::from(format!("{anchor}/Actions/Oem/{}", cycle.action));
    cx.post(&target, &(cycle.payload)()).await
}

async fn chassis<B: Bmc>(
    cx: &OpCx<'_, B>,
    chassis_id: &str,
) -> Result<nv_redfish::chassis::Chassis<B>, PlatformError> {
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

pub(super) async fn chassis_reset<B: Bmc>(
    cx: &OpCx<'_, B>,
    chassis_id: &str,
    reset_type: ResetType,
) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    already_satisfied_is_complete(
        chassis(cx, chassis_id)
            .await?
            .reset(Some(reset_type))
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error)),
    )
}

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

#[async_trait]
impl<B> Power<B> for StandardPower
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn state(&self, cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
        state(cx)
    }

    async fn ac_power_cycle_supported(&self, _cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        Ok(self.full_power_cycle.is_some())
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        match reset_type {
            ResetType::FullPowerCycle => match &self.full_power_cycle {
                Some(cycle) => oem_power_cycle(cx, cycle).await,
                None => reset(cx, reset_type).await,
            },
            ResetType::ForceRestart | ResetType::GracefulRestart
                if self.restart == Restart::Ipmi =>
            {
                ipmi_restart(cx).await
            }
            other => reset(cx, other).await,
        }
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

        let outcome = STANDARD_POWER
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
