/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use serde_json::json;

use crate::bmc_control::standard::StandardBmcControl;
use crate::bmc_control::support::manager_oem_action;

/// Supermicro restores manager defaults through the OEM `SmcManagerConfig.Reset` action.
pub(crate) struct SmcBmcControl;

#[async_trait]
impl<B: Bmc> BmcControl<B> for SmcBmcControl {
    fn standard(&self) -> &dyn BmcControl<B> {
        &StandardBmcControl
    }

    async fn reset_to_factory_defaults(
        &self,
        cx: &OpCx<'_, B>,
    ) -> Result<DriverOutcome, PlatformError> {
        manager_oem_action(
            cx,
            "SmcManagerConfig.Reset",
            &json!({"Option": "ClearConfig"}),
        )
        .await
    }
}
