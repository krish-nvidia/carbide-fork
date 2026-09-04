/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, Fetched, OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc, EntityTypeRef, ODataId};
use nv_redfish::resource::{PowerState, ResetType};
use serde::Deserialize;
use serde_json::json;

use crate::power::standard::{self, power_state_from_supplies, power_supplies};

/// Delta power-shelf power behavior.
///
/// The shelf has no ComputerSystem; its PSUs are switched through OEM actions
/// on the PowerShelf resource and only support on and off.
pub(crate) struct DeltaPowerShelfPower;

/// The PowerShelf OEM actions nv-redfish does not model.
#[derive(Deserialize)]
struct Shelf {
    #[serde(rename = "Oem")]
    oem: Option<ShelfOem>,
}

#[derive(Deserialize)]
struct ShelfOem {
    deltaenergysystems: Option<DeltaOem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DeltaOem {
    actions: Option<DeltaActions>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DeltaActions {
    #[serde(rename = "#PowerShelf.TurnOnPSUs")]
    turn_on_psus: Option<ActionTarget>,
    #[serde(rename = "#PowerShelf.TurnOffPSUs")]
    turn_off_psus: Option<ActionTarget>,
}

#[derive(Deserialize)]
struct ActionTarget {
    target: String,
}

async fn set_psus<B: Bmc>(cx: &OpCx<'_, B>, on: bool) -> Result<DriverOutcome, PlatformError> {
    let shelf = cx
        .service_root()
        .power_equipment()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .power_shelves()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .next()
        .ok_or(PlatformError::Unsupported)?;
    let shelf = cx
        .bmc()
        .get::<Fetched<Shelf>>(shelf.raw().odata_id())
        .await
        .map_err(|error| cx.map_bmc_error(error))?;
    let actions = shelf
        .oem
        .as_ref()
        .and_then(|oem| oem.deltaenergysystems.as_ref())
        .and_then(|delta| delta.actions.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    let action = if on {
        actions.turn_on_psus.as_ref()
    } else {
        actions.turn_off_psus.as_ref()
    };
    let target = ODataId::from(action.ok_or(PlatformError::Unsupported)?.target.clone());
    cx.post(&target, &json!({})).await
}

#[async_trait]
impl<B> Power<B> for DeltaPowerShelfPower
where
    B: Bmc,
    B::Error: ActionError,
{
    async fn state(&self, cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
        let mut states = Vec::new();
        for supply in power_supplies(cx).await? {
            states.push(
                supply
                    .oem_delta()
                    .map_err(|error| cx.map_redfish_error(error))?
                    .and_then(|delta| delta.power()),
            );
        }
        power_state_from_supplies(&states)
    }

    async fn ac_power_cycle_supported(&self, _cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        Ok(false)
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        match reset_type {
            ResetType::On => set_psus(cx, true).await,
            ResetType::ForceOff | ResetType::GracefulShutdown => set_psus(cx, false).await,
            _ => Err(PlatformError::Unsupported),
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
