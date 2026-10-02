/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::BootUpdate;

use crate::boot_order::standard::{StandardBootOrder, settings_object_override};

/// NVIDIA DGX Viking boot behavior; boot overrides go through the system's
/// pending-settings resource, and an override without a mode boots UEFI.
pub(crate) struct VikingBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for VikingBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        settings_object_override(cx, override_setting).await
    }
}
