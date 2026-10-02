/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::sync::Arc;

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::{AccountServiceConfig, ManagerAccountCreate};
use nv_redfish::core::Bmc;
use nv_redfish::oem::dell::IdracVersion;
use nv_redfish::schema::manager_account::ManagerAccount;

use crate::accounts::standard::StandardAccounts;
use crate::accounts::support::{create_account, delete_account, list_accounts};

/// Dell iDRAC account behavior.
///
/// iDRAC8 and iDRAC9 accounts are fixed slots: creation enables a free slot
/// rather than POSTing to the collection, deletion disables the slot, and
/// listing hides disabled slots. Every password-policy property is read-only.
pub(crate) struct IdracAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for IdracAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn list(&self, cx: &OpCx<'_, B>) -> Result<Vec<Arc<ManagerAccount>>, PlatformError> {
        list_accounts(cx, slot_config(cx).await?).await
    }

    async fn create(
        &self,
        cx: &OpCx<'_, B>,
        request: ManagerAccountCreate,
    ) -> Result<DriverOutcome, PlatformError> {
        create_account(cx, slot_config(cx).await?, request).await
    }

    async fn delete(
        &self,
        cx: &OpCx<'_, B>,
        username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        delete_account(cx, slot_config(cx).await?, username).await
    }

    async fn apply_default_policy(
        &self,
        _cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }
}

/// The account behavior of this iDRAC's version; an unrecognized manager
/// model keeps the iDRAC9 fixed slots.
async fn slot_config<B: Bmc>(cx: &OpCx<'_, B>) -> Result<AccountServiceConfig, PlatformError> {
    Ok(IdracVersion::from_manager(cx.manager().await?)
        .unwrap_or(IdracVersion::IDRAC9)
        .account_service_config())
}
