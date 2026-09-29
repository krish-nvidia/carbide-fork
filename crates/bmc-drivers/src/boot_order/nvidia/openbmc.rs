/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::BootUpdate;

use crate::boot_order::standard::StandardBootOrder;

/// NVIDIA OpenBMC boot behavior; boot overrides are accepted only through the
/// system's pending-settings resource, not the live system.
pub(crate) struct OpenBmcBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for OpenBmcBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        cx.system()?
            .set_boot_source_override(
                override_setting
                    .boot_source_override_target
                    .ok_or(PlatformError::Unsupported)?,
                override_setting
                    .boot_source_override_enabled
                    .ok_or(PlatformError::Unsupported)?,
                override_setting.boot_source_override_mode,
                override_setting.http_boot_uri.clone(),
            )
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }
}
