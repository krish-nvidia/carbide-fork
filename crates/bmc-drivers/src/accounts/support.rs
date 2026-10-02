/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Account-service operations shared by the account drivers.

use std::sync::Arc;

use bmc_platform::{AuthError, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::{
    Account, AccountCollection, AccountService, AccountServiceConfig, AccountServiceUpdate,
    ManagerAccountCreate, ManagerAccountUpdate,
};
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::schema::manager_account::ManagerAccount;
use serde::de::IgnoredAny;

/// The standard account collection, addressed directly when the BMC refuses
/// the reads that would discover it.
const ACCOUNTS: &str = "/redfish/v1/AccountService/Accounts";

/// Every account, as `config` lists them.
pub(super) async fn list_accounts<B: Bmc>(
    cx: &OpCx<'_, B>,
    config: AccountServiceConfig,
) -> Result<Vec<Arc<ManagerAccount>>, PlatformError> {
    let accounts = accounts(cx, config)
        .await?
        .all_accounts_data()
        .await
        .map_err(|error| cx.map_redfish_error(error))?;
    Ok(accounts.into_iter().map(|account| account.raw()).collect())
}

/// Creates an account the way `config` prescribes.
pub(super) async fn create_account<B: Bmc>(
    cx: &OpCx<'_, B>,
    config: AccountServiceConfig,
    request: ManagerAccountCreate,
) -> Result<DriverOutcome, PlatformError> {
    accounts(cx, config)
        .await?
        .create_account(request)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Deletes the named account the way `config` prescribes.
pub(super) async fn delete_account<B: Bmc>(
    cx: &OpCx<'_, B>,
    config: AccountServiceConfig,
    username: &str,
) -> Result<DriverOutcome, PlatformError> {
    account_by_username(cx, config, username)
        .await?
        .delete()
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Sets the named account's password.
///
/// A BMC in factory state refuses the account lookup with
/// `PasswordChangeRequired`, and until the change it refuses everything
/// else too. The password is then set on the account the BMC names, or on
/// `factory_account_id` when it names none.
pub(super) async fn set_password<B: Bmc>(
    cx: &OpCx<'_, B>,
    username: &str,
    password: &str,
    factory_account_id: Option<&str>,
) -> Result<DriverOutcome, PlatformError> {
    match account_by_username(cx, AccountServiceConfig::standard(), username).await {
        Ok(account) => account
            .update_password(password.to_string())
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error)),
        Err(PlatformError::Auth(AuthError::PasswordChangeRequired { account_uri })) => {
            match account_uri.or_else(|| factory_account_id.map(|id| format!("{ACCOUNTS}/{id}"))) {
                Some(account_uri) => set_required_password(cx, account_uri, password).await,
                None => Err(PlatformError::Auth(AuthError::PasswordChangeRequired {
                    account_uri: None,
                })),
            }
        }
        Err(error) => Err(error),
    }
}

/// Renames the named account.
pub(super) async fn rename_account<B: Bmc>(
    cx: &OpCx<'_, B>,
    old_username: &str,
    new_username: &str,
) -> Result<DriverOutcome, PlatformError> {
    account_by_username(cx, AccountServiceConfig::standard(), old_username)
        .await?
        .update_user_name(new_username.to_string())
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Updates account-service properties, typically a lockout policy.
pub(super) async fn apply_policy<B: Bmc>(
    cx: &OpCx<'_, B>,
    body: AccountServiceUpdate,
) -> Result<DriverOutcome, PlatformError> {
    account_service(cx, AccountServiceConfig::standard())
        .await?
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
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

async fn accounts<B: Bmc>(
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

async fn account_by_username<B: Bmc>(
    cx: &OpCx<'_, B>,
    config: AccountServiceConfig,
    username: &str,
) -> Result<Account<B>, PlatformError> {
    accounts(cx, config)
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

/// PATCHes the password straight onto `account_uri`, without the reads a
/// BMC in factory state refuses.
async fn set_required_password<B: Bmc>(
    cx: &OpCx<'_, B>,
    account_uri: String,
    password: &str,
) -> Result<DriverOutcome, PlatformError> {
    cx.bmc()
        .update::<_, IgnoredAny>(
            &ODataId::from(account_uri),
            None,
            &ManagerAccountUpdate::builder()
                .with_password(password.to_string())
                .build(),
        )
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_bmc_error(error))
}
