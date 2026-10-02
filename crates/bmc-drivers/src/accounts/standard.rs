/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Standard Redfish account operations.

use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{Accounts, AuthError, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::{
    Account, AccountCollection, AccountService, AccountServiceConfig, AccountServiceUpdate,
    ManagerAccountCreate, ManagerAccountUpdate,
};
use nv_redfish::core::{Bmc, ODataId};
use nv_redfish::schema::manager_account::ManagerAccount;
use serde::de::IgnoredAny;

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
        let account = match account_by_username(cx, username).await {
            Ok(account) => account,
            Err(PlatformError::Auth(AuthError::PasswordChangeRequired {
                account_uri: Some(account_uri),
            })) => return change_required_password(cx, account_uri, password).await,
            Err(error) => return Err(error),
        };
        account
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

/// Sets the password of the account a `PasswordChangeRequired` response
/// named. Until that change the BMC refuses everything else, including the
/// account-service reads a lookup needs.
async fn change_required_password<B: Bmc>(
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

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::*;
    use crate::test_support::{Fixture, FixtureBmc, body, path};

    #[tokio::test]
    async fn factory_password_change_patches_the_account_the_bmc_names() {
        let bmc = factory_bmc().await;
        let cx = bmc.lazy_cx();

        assert_eq!(
            StandardAccounts
                .change_password(&cx, "root", "new-password")
                .await,
            Ok(DriverOutcome::complete())
        );
        let writes = bmc.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].method, Method::PATCH);
        assert_eq!(path(&writes[0]), ACCOUNT);
        assert_eq!(body(&writes[0]), json!({"Password": "new-password"}));
        // AMI firmware rejects the first password change without `If-Match: *`.
        assert_eq!(
            writes[0]
                .headers
                .get("If-Match")
                .and_then(|value| value.to_str().ok()),
            Some("*")
        );
    }

    const ACCOUNT: &str = "/redfish/v1/AccountService/Accounts/root";

    /// A BMC in factory state: every read but the service root answers
    /// `PasswordChangeRequired` naming the account to change.
    async fn factory_bmc() -> FixtureBmc {
        let password_change_required = json!({"error": {
            "code": "Base.1.18.1.GeneralError",
            "@Message.ExtendedInfo": [{
                "MessageId": "Base.1.18.1.PasswordChangeRequired",
                "MessageArgs": [ACCOUNT]
            }]
        }});
        Fixture::new("NVIDIA", "GB200 NVL", "System_0", "BMC_0")
            .document(
                "/redfish/v1",
                json!({
                    "@odata.id": "/redfish/v1",
                    "Id": "RootService",
                    "Name": "Root Service",
                    "Systems": {"@odata.id": "/redfish/v1/Systems"},
                    "AccountService": {"@odata.id": "/redfish/v1/AccountService"},
                    "Links": {"Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}},
                }),
            )
            .respond(
                Method::GET,
                "/redfish/v1/Systems",
                StatusCode::FORBIDDEN,
                Some(password_change_required.clone()),
            )
            .respond(
                Method::GET,
                "/redfish/v1/AccountService",
                StatusCode::FORBIDDEN,
                Some(password_change_required),
            )
            .build()
            .await
    }
}
