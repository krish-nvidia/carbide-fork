/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::accounts::standard::{StandardAccounts, apply_policy};
use crate::accounts::support::openbmc_minimum_lockout_policy;

/// NVIDIA GB NVSwitch trays apply the OpenBMC lockout policy.
pub(crate) struct SwitchAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for SwitchAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        apply_policy(cx, &openbmc_minimum_lockout_policy()).await
    }
}
