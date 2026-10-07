/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::AccountServiceUpdate;
use nv_redfish::core::Bmc;
use nv_redfish::oem::lenovo::account_service::{
    LenovoAccountServiceUpdate, LenovoAccountServiceUpdateExt,
};

use crate::accounts::standard::StandardAccounts;
use crate::accounts::support::RedfishAccountsExt as _;

/// Lenovo XCC rejects a zero lockout duration and enforces password rotation
/// through `Oem.Lenovo`, which the default policy disables. A factory-state
/// demand for a password change without an account means the factory
/// administrator, account 1.
pub(crate) struct XccAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for XccAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn change_password(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
        password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        cx.set_account_password(username, password, Some("1")).await
    }

    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        let policy = AccountServiceUpdate::builder()
            .with_account_lockout_threshold(0)
            .with_account_lockout_duration(60)
            .build()
            .with_oem_lenovo(
                LenovoAccountServiceUpdate::builder()
                    .with_password_expiration_period_days(0.0)
                    .with_password_change_on_first_access(false)
                    .with_minimum_password_change_interval_hours(0.0)
                    .with_minimum_password_reuse_cycle(0.0)
                    .with_password_expiration_warning_period(0.0)
                    .build(),
            )
            .map_err(|error| PlatformError::InvalidResponse {
                message: format!("failed to build the Lenovo account policy: {error}"),
            })?;
        cx.apply_account_policy(policy).await
    }
}
