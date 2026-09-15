/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::resource::ResetType;
use serde_json::json;

use crate::power::standard::StandardPower;

/// Supermicro power behavior: AC power is restored through the OEM system reset.
pub(crate) struct SmcPower;

/// Restores AC power through the Supermicro OEM system reset.
async fn ac_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
    let target = ODataId::from(format!(
        "{}/Actions/Oem/OemSystemExtensions.Reset",
        cx.system()?.odata_id()
    ));
    cx.post(&target, &json!({"ResetType": "ACCycle"})).await
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
