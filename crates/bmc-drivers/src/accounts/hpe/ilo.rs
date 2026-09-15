/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Accounts, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use serde_json::json;

use crate::accounts::standard::{StandardAccounts, apply_policy};

/// HPE iLO: the lockout policy lives under `Oem.Hpe`.
pub(crate) struct IloAccounts;

#[async_trait]
impl<B: Bmc> Accounts<B> for IloAccounts {
    fn standard(&self) -> &dyn Accounts<B> {
        &StandardAccounts
    }

    async fn apply_default_policy(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        apply_policy(
            cx,
            &json!({
                "Oem": {
                    "Hpe": {
                        "AuthFailureDelayTimeSeconds": 2,
                        "AuthFailureLoggingThreshold": 0,
                        "AuthFailuresBeforeDelay": 0,
                        "EnforcePasswordComplexity": false
                    }
                }
            }),
        )
        .await
    }
}
