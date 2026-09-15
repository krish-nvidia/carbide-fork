/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{Bmc, RedfishSettings};
use nv_redfish::schema::computer_system::{BootUpdate, ComputerSystem as ComputerSystemSchema};
use serde_json::json;

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
        let system = cx.system()?;
        let settings = system
            .raw()
            .settings_object()
            .ok_or(PlatformError::Unsupported)?
            .get(cx.bmc())
            .await
            .map_err(|error| cx.map_bmc_error(error))?;
        let settings: &ComputerSystemSchema = &settings;
        cx.patch(settings, &json!({"Boot": override_setting})).await
    }
}
