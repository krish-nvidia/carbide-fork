/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::sync::Arc;

use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::ManagerAccountCreate;
use nv_redfish::core::{Bmc, EntityTypeRef};
use nv_redfish::schema::manager_account::ManagerAccount;
use serde_json::{Value, json};

use crate::accounts::standard;

/// iDRAC exposes fixed account slots; slot 1 is reserved and slot 2 is `root`.
const FIRST_USER_SLOT: u8 = 3;
const LAST_USER_SLOT: u8 = 16;

/// Dell iDRAC account behavior.
///
/// Accounts are created by enabling a free slot rather than POSTing to the
/// collection, and the password-policy properties are read-only.
pub(crate) struct IdracAccounts;

#[async_trait::async_trait]
impl<B: Bmc> Accounts<B> for IdracAccounts {
    async fn list(&self, cx: &OpCx<'_, B>) -> Result<Vec<Arc<ManagerAccount>>, PlatformError> {
        standard::list(cx).await
    }

    async fn create(
        &self,
        cx: &OpCx<'_, B>,
        request: &ManagerAccountCreate,
    ) -> Result<DriverOutcome, PlatformError> {
        create_in_disabled_slot(cx, request).await
    }

    async fn delete(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::delete(cx, username).await
    }

    async fn change_password(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
        password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::update_account(cx, username, json!({ "Password": password })).await
    }

    async fn change_username(
        &self,
        cx: &OpCx<'_, B>,
        old_username: &str,
        new_username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::update_account(cx, old_username, json!({ "UserName": new_username })).await
    }

    async fn apply_default_policy(
        &self,
        _cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

async fn create_in_disabled_slot<B: Bmc>(
    cx: &OpCx<'_, B>,
    request: &ManagerAccountCreate,
) -> Result<DriverOutcome, PlatformError> {
    let slot = |account: &nv_redfish::account::Account<B>| account.raw().base.id.parse::<u8>().ok();
    let account = standard::account_collection(cx)
        .await?
        .all_accounts_data()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .into_iter()
        .filter(|account| {
            slot(account).is_some_and(|slot| (FIRST_USER_SLOT..=LAST_USER_SLOT).contains(&slot))
                && account.raw().enabled == Some(false)
        })
        .min_by_key(slot)
        .ok_or(PlatformError::TooManyUsers)?;
    let raw = account.raw();
    let mut payload =
        serde_json::to_value(request).map_err(|error| PlatformError::InvalidResponse {
            message: format!("failed to serialize account request: {error}"),
        })?;
    payload["Enabled"] = Value::Bool(true);
    cx.patch_id(raw.odata_id(), raw.etag(), &payload)
        .await
        .map(DriverOutcome::from)
}
