/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use serde_json::json;

use crate::bios::attributes::BiosAttribute;
use crate::bios::attributes::lenovo::{ami, gb300};
use crate::bios::standard::{self, StandardBios};
use crate::resources::{patch_bios_attributes, selected_bios};

/// AMI MegaRAC (including Lenovo AMI-based systems) names the UEFI
/// administrator password `SETUP001`; the model decides the attributes machine
/// setup expects.
pub(crate) struct MegaRacBios {
    attributes: &'static [BiosAttribute],
}

impl MegaRacBios {
    /// AMI MegaRAC BIOS, including Lenovo HS350x-class systems.
    pub(crate) const AMI: Self = Self {
        attributes: ami::ATTRIBUTES,
    };
    /// Lenovo GB300, whose values are prefixed with their attribute id.
    pub(crate) const LENOVO_GB300: Self = Self {
        attributes: gb300::ATTRIBUTES,
    };
}

const UEFI_PASSWORD_NAME: &str = "SETUP001";

/// The BIOS attribute that requests a TPM operation on the next boot.
const TPM_OPERATION: &str = "TCG006";

#[async_trait]
impl<B: Bmc> Bios<B> for MegaRacBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn expected(
        &self,
        cx: &OpCx<'_, B>,
        overlay: &BiosSettings,
    ) -> Result<BiosSettings, PlatformError> {
        standard::expected(cx, self.attributes, overlay).await
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        standard::change_password(cx, UEFI_PASSWORD_NAME, current_password, "").await
    }

    /// GB300's Grace BIOS prefixes `TCG006` enum values with the attribute id.
    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        let prefixed = selected_bios(cx)
            .await?
            .attribute(TPM_OPERATION)
            .and_then(|value| {
                value
                    .str_value()
                    .map(|value| value.starts_with(TPM_OPERATION))
            })
            .unwrap_or(false);
        let clear = if prefixed {
            "TCG006TPMClear"
        } else {
            "TPM Clear"
        };
        patch_bios_attributes(cx, json!({TPM_OPERATION: clear})).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, body, path};

    const SYSTEM: &str = "/redfish/v1/Systems/Self";
    const BIOS: &str = "/redfish/v1/Systems/Self/Bios";
    const SETTINGS: &str = "/redfish/v1/Systems/Self/Bios/SD";

    #[tokio::test]
    async fn tpm_clear_uses_the_enum_spelling_the_bios_reports() {
        for (current, clear) in [("TCG006None", "TCG006TPMClear"), ("None", "TPM Clear")] {
            let bmc = Fixture::new("AMI", "AMI Redfish Server", "Self", "Self")
                .document(
                    SYSTEM,
                    json!({"@odata.id": SYSTEM, "Id": "Self", "Name": "System", "Bios": {"@odata.id": BIOS}}),
                )
                .document(
                    BIOS,
                    json!({
                        "@odata.id": BIOS,
                        "@Redfish.Settings": {"SettingsObject": {"@odata.id": SETTINGS}},
                        "Id": "BIOS",
                        "Name": "BIOS",
                        "Attributes": {TPM_OPERATION: current},
                    }),
                )
                .document(
                    SETTINGS,
                    json!({"@odata.id": SETTINGS, "Id": "SD", "Name": "BIOS Settings", "Attributes": {}}),
                )
                .build()
                .await;
            let cx = bmc.cx().await;

            MegaRacBios::AMI
                .clear_tpm(&cx)
                .await
                .expect("TPM clear is staged");
            let writes = bmc.writes();
            assert_eq!(writes.len(), 1, "{current}");
            assert_eq!(path(&writes[0]), SETTINGS, "{current}");
            assert_eq!(
                body(&writes[0]),
                json!({"Attributes": {TPM_OPERATION: clear}}),
                "{current}"
            );
        }
    }
}
