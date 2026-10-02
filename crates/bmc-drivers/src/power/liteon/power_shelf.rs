/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{OpCx, PlatformError, Power};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::resource::PowerState;

use crate::power::standard::StandardPower;
use crate::power::support::power_state_from_supplies;

/// Lite-On power-shelf power behavior.
///
/// The shelf's ComputerSystem accepts standard resets, but its power state
/// is only meaningful as the aggregate of the supplies' OEM `PowerState`.
pub(crate) struct LiteOnPowerShelfPower;

#[async_trait]
impl<B: Bmc> Power<B> for LiteOnPowerShelfPower
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Power<B> {
        &StandardPower
    }

    async fn state(&self, cx: &OpCx<'_, B>) -> Result<PowerState, PlatformError> {
        let chassis = cx
            .service_root()
            .chassis()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .ok_or(PlatformError::Unsupported)?
            .members()
            .await
            .map_err(|error| cx.map_redfish_error(error))?;
        let mut states = Vec::new();
        for chassis in &chassis {
            let Some(supplies) = chassis
                .oem_liteon_power_supply_links()
                .await
                .map_err(|error| cx.map_redfish_error(error))?
            else {
                continue;
            };
            for supply in supplies {
                let supply = supply
                    .fetch()
                    .await
                    .map_err(|error| cx.map_redfish_error(error))?;
                states.push(supply.power_state);
            }
        }
        power_state_from_supplies(&states)
    }
}

#[cfg(test)]
mod tests {
    use bmc_platform::PlatformIdentity;

    use super::*;

    #[tokio::test]
    async fn state_reads_the_liteon_supply_power_states() {
        let bmc = bmc_mock::test_support::liteon_powershelf_bmc().await;
        let identity = PlatformIdentity::default();
        let cx = OpCx::new(bmc.bmc.as_ref(), bmc.service_root.as_ref(), &identity)
            .await
            .expect("power shelf context resolves");

        assert_eq!(LiteOnPowerShelfPower.state(&cx).await, Ok(PowerState::On));
    }
}
