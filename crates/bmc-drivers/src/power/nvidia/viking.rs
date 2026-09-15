/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Power};
use nv_redfish::core::Bmc;
use nv_redfish::resource::ResetType;

use crate::power::standard::StandardPower;
use crate::power::support::ipmi_restart;

/// NVIDIA DGX Viking cuts power to its DPUs on a Redfish restart, so the host
/// restarts over IPMI; the AMI firmware offers no AC power cycle.
pub(crate) struct VikingPower;

#[async_trait]
impl<B: Bmc> Power<B> for VikingPower {
    fn standard(&self) -> &dyn Power<B> {
        &StandardPower
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        match reset_type {
            ResetType::ForceRestart | ResetType::GracefulRestart => ipmi_restart(cx).await,
            other => self.standard().set(cx, other).await,
        }
    }
}
