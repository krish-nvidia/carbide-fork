/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::Bmc;
use nv_redfish::oem::supermicro::SupermicroSystemResetType;
use nv_redfish::oem::supermicro::schema::ActionAnnotations;
use nv_redfish::oem::supermicro::schema::oem_system_extensions::ComputerSystemResetAction;
use nv_redfish::resource::ResetType;

use crate::power::standard::StandardPower;

/// Supermicro power behavior: AC power is restored through the OEM system reset.
pub(crate) struct SmcPower;

/// Restores AC power through the Supermicro OEM system reset.
async fn ac_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let actions = cx
        .system()?
        .oem_supermicro_actions()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .raw();
    let action = actions.reset.as_ref().ok_or(PlatformError::Unsupported)?;
    cx.action(
        action,
        &ComputerSystemResetAction {
            redfish_annotations: ActionAnnotations::default(),
            reset_type: SupermicroSystemResetType::AcCycle,
        },
    )
    .await
}

#[async_trait]
impl<B: Bmc> Power<B> for SmcPower {
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
            ResetType::FullPowerCycle => ac_power_cycle(cx).await,
            other => self.standard().set(cx, other).await,
        }
    }
}
