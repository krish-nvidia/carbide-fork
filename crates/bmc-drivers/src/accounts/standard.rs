/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish account operations.

use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::{
    Account, AccountCollection, AccountService, AccountServiceConfig, AccountServiceUpdate,
    ManagerAccountCreate,
};
use nv_redfish::core::Bmc;
use nv_redfish::schema::manager_account::ManagerAccount;

/// Redfish-standard account operations.
pub(crate) struct StandardAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for StandardAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        self
    }

    async fn list(&self, cx: &OpCx<'_, B>) -> Result<Vec<Arc<ManagerAccount>>, PlatformError> {
        let accounts = account_collection(cx)
            .await?
            .all_accounts_data()
            .await
            .map_err(|error| cx.map_redfish_error(error))?;
        Ok(accounts.into_iter().map(|account| account.raw()).collect())
    }

    async fn create(
        &self,
        cx: &OpCx<'_, B>,
        request: ManagerAccountCreate,
    ) -> Result<DriverOutcome, PlatformError> {
        account_collection(cx)
            .await?
            .create_account(request)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }

    async fn delete(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        account_by_username(cx, username)
            .await?
            .delete()
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }

    async fn change_password(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
        password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        account_by_username(cx, username)
            .await?
            .update_password(password.to_string())
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }

    async fn change_username(
        &self,
        cx: &OpCx<'_, B>,
        old_username: &str,
        new_username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        account_by_username(cx, old_username)
            .await?
            .update_user_name(new_username.to_string())
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }

    /// Disables account lockout so NICo cannot lock itself out during automation.
    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        apply_policy(
            cx,
            AccountServiceUpdate::builder()
                .with_account_lockout_threshold(0)
                .with_account_lockout_duration(0)
                .with_account_lockout_counter_reset_after(0)
                .build(),
        )
        .await
    }
}

async fn account_service<B: Bmc>(
    cx: &OpCx<'_, B>,
    config: AccountServiceConfig,
) -> Result<AccountService<B>, PlatformError> {
    cx.service_root()
        .account_service(config)
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// The account collection, with accounts created, listed, and deleted as
/// `config` prescribes.
pub(super) async fn accounts_with<B: Bmc>(
    cx: &OpCx<'_, B>,
    config: AccountServiceConfig,
) -> Result<AccountCollection<B>, PlatformError> {
    account_service(cx, config)
        .await?
        .accounts()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// The account service's account collection.
async fn account_collection<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<AccountCollection<B>, PlatformError> {
    accounts_with(cx, AccountServiceConfig::standard()).await
}

/// Updates account-service properties, typically a lockout policy.
pub(super) async fn apply_policy<B: Bmc>(
    cx: &OpCx<'_, B>,
    body: AccountServiceUpdate,
) -> Result<DriverOutcome, PlatformError> {
    let service = account_service(cx, AccountServiceConfig::standard()).await?;
    service
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

async fn account_by_username<B: Bmc>(
    cx: &OpCx<'_, B>,
    username: &str,
) -> Result<Account<B>, PlatformError> {
    account_collection(cx)
        .await?
        .all_accounts_data()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .find(|account| account.raw().user_name.as_deref() == Some(username))
        .ok_or_else(|| PlatformError::UserNotFound {
            identifier: username.to_string(),
        })
}
