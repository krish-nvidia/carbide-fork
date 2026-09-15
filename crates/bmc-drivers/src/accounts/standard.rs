/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish account operations.

use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::{Account, AccountCollection, ManagerAccountCreate};
use nv_redfish::core::Bmc;
use nv_redfish::schema::manager_account::ManagerAccount;
use serde_json::{Value, json};

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
        request: &ManagerAccountCreate,
    ) -> Result<DriverOutcome, PlatformError> {
        let collection = account_collection(cx).await?;
        cx.post(collection.odata_id(), request).await
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
        update_account(cx, username, json!({ "Password": password })).await
    }

    async fn change_username(
        &self,
        cx: &OpCx<'_, B>,
        old_username: &str,
        new_username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        update_account(cx, old_username, json!({ "UserName": new_username })).await
    }

    /// Disables account lockout so NICo cannot lock itself out during automation.
    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        apply_policy(
            cx,
            &json!({
                "AccountLockoutThreshold": 0,
                "AccountLockoutDuration": 0,
                "AccountLockoutCounterResetAfter": 0
            }),
        )
        .await
    }
}

/// The account service's account collection.
pub(super) async fn account_collection<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<AccountCollection<B>, PlatformError> {
    cx.service_root()
        .account_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .accounts()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// Patches account-service properties, typically a lockout policy.
pub(super) async fn apply_policy<B: Bmc>(
    cx: &OpCx<'_, B>,
    payload: &Value,
) -> Result<DriverOutcome, PlatformError> {
    let service = cx
        .service_root()
        .account_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?;
    cx.patch(service.raw().as_ref(), payload).await
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

async fn update_account<B: Bmc>(
    cx: &OpCx<'_, B>,
    username: &str,
    payload: Value,
) -> Result<DriverOutcome, PlatformError> {
    let account = account_by_username(cx, username).await?;
    let raw = account.raw();
    cx.patch(raw.as_ref(), &payload).await
}
