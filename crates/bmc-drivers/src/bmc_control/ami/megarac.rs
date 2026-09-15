/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::resource::ResetType;

use crate::bmc_control::standard::{self, StandardBmcControl};

/// AMI MegaRAC accepts at most two NTP servers and only restarts through `ForceRestart`.
pub(crate) struct MegaRacBmcControl;

const MAX_NTP_SERVERS: usize = 2;

#[async_trait]
impl<B: Bmc> BmcControl<B> for MegaRacBmcControl {
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        standard::reset(cx, ResetType::ForceRestart).await
    }

    async fn set_ntp_servers(
        &self,
        cx: &OpCx<'_, B>,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        self.standard()
            .set_ntp_servers(cx, &servers[..servers.len().min(MAX_NTP_SERVERS)])
            .await
    }
}
