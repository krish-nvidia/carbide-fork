/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::accounts::standard::StandardAccounts;
use crate::accounts::support::RedfishAccountsExt as _;

/// Generic AMI MegaRAC: a factory-state BMC that demands a password change
/// without naming the account means the administrator, account 2.
pub(crate) struct MegaRacAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for MegaRacAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn change_password(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
        password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        cx.set_account_password(username, password, Some("2")).await
    }
}
