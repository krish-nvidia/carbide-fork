/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BmcControl, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::oem::supermicro::ResetOption;
use nv_redfish::oem::supermicro::schema::ActionAnnotations;
use nv_redfish::oem::supermicro::schema::manager::OemActions;
use nv_redfish::oem::supermicro::schema::smc_manager_config::ManagerResetAction;
use serde::Deserialize;

use crate::bmc_control::standard::StandardBmcControl;

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
        let manager = cx.manager()?.raw();
        let actions = manager
            .actions
            .as_ref()
            .and_then(|actions| actions.oem.as_ref())
            .map(|oem| OemActions::deserialize(&oem.additional_properties))
            .transpose()
            .map_err(|error| PlatformError::InvalidResponse {
                message: format!("Supermicro manager actions are malformed: {error}"),
            })?;
        let action = actions
            .as_ref()
            .and_then(|actions| actions.reset.as_ref())
            .ok_or(PlatformError::Unsupported)?;
        cx.action(
            action,
            &ManagerResetAction {
                redfish_annotations: ActionAnnotations::default(),
                option: ResetOption::ClearConfig,
            },
        )
        .await
    }
}
