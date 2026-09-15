/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::{BootSource, BootUpdate};
use serde_json::json;

use crate::boot_order::standard::StandardBootOrder;
use crate::resources::patch_bios_attributes;

/// HPE iLO boot behavior; a UEFI HTTP override is pinned through BIOS attributes.
pub(crate) struct IloBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for IloBootOrder {
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        if override_setting.boot_source_override_target != Some(BootSource::UefiHttp) {
            return Err(PlatformError::Unsupported);
        }
        let uri = override_setting
            .http_boot_uri
            .as_deref()
            .ok_or(PlatformError::Unsupported)?;
        patch_bios_attributes(cx, json!({"UrlBootFile": uri, "PreBootNetwork": "IPv4"})).await
    }
}
