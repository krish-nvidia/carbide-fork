/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::{BootSourceOverrideMode, BootUpdate};

use crate::boot_order::standard::StandardBootOrder;

/// NVIDIA BlueField boot behavior; boot overrides go through the system's
/// pending-settings resource, and an override without a mode boots UEFI.
pub(crate) struct BlueFieldBootOrder;

#[async_trait]
impl<B: Bmc> BootOrder<B> for BlueFieldBootOrder {
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
                Some(
                    override_setting
                        .boot_source_override_mode
                        .unwrap_or(BootSourceOverrideMode::Uefi),
                ),
                override_setting.http_boot_uri.clone(),
            )
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::header::IF_MATCH;
    use nv_redfish::schema::computer_system::{BootSource, BootSourceOverrideEnabled};
    use serde_json::json;

    use super::*;
    use crate::test_support::{Fixture, body, path};

    #[tokio::test]
    async fn override_without_a_mode_boots_uefi_through_the_settings_object() {
        let bmc = Fixture::new("Nvidia", "BlueField-3 DPU", "Bluefield", "Bluefield_BMC")
            .document(
                "/redfish/v1/Systems/Bluefield",
                json!({
                    "@odata.id": "/redfish/v1/Systems/Bluefield",
                    "@odata.etag": "\"system-1\"",
                    "@Redfish.Settings": {
                        "SettingsObject": {"@odata.id": "/redfish/v1/Systems/Bluefield/Settings"}
                    },
                    "Id": "Bluefield",
                    "Name": "Bluefield",
                }),
            )
            .build()
            .await;
        let cx = bmc.cx().await;

        BlueFieldBootOrder
            .set_override(
                &cx,
                &BootUpdate::builder()
                    .with_boot_source_override_target(BootSource::UefiHttp)
                    .with_boot_source_override_enabled(BootSourceOverrideEnabled::Once)
                    .build(),
            )
            .await
            .expect("override succeeds");

        let writes = bmc.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(path(&writes[0]), "/redfish/v1/Systems/Bluefield/Settings");
        assert_eq!(writes[0].headers[IF_MATCH], "*");
        assert_eq!(
            body(&writes[0]),
            json!({"Boot": {
                "BootSourceOverrideTarget": "UefiHttp",
                "BootSourceOverrideEnabled": "Once",
                "BootSourceOverrideMode": "UEFI",
            }})
        );
    }
}
