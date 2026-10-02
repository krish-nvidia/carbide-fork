/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::account::ManagerAccountCreate;
use nv_redfish::core::Bmc;
use nv_redfish::oem::dell::IdracVersion;

use crate::accounts::standard::{self, StandardAccounts};

/// Dell iDRAC account behavior.
///
/// iDRAC8 and iDRAC9 create accounts by enabling a free fixed slot rather
/// than POSTing to the collection, and every password-policy property is
/// read-only.
pub(crate) struct IdracAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for IdracAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    /// An unrecognized manager model keeps the iDRAC9 fixed slots.
    async fn create(
        &self,
        cx: &OpCx<'_, B>,
        request: ManagerAccountCreate,
    ) -> Result<DriverOutcome, PlatformError> {
        let version =
            IdracVersion::from_manager(cx.manager().await?).unwrap_or(IdracVersion::IDRAC9);
        standard::accounts_with(cx, version.account_service_config())
            .await?
            .create_account(request)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }

    async fn apply_default_policy(
        &self,
        _cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }
}
