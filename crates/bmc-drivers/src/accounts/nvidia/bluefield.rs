/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::accounts::standard::StandardAccounts;

/// NVIDIA BlueField: the DPU BMC exposes its lockout policy read-only.
pub(crate) struct BlueFieldAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for BlueFieldAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn apply_default_policy(
        &self,
        _cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }
}
