/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    Bios, BiosSettings, BiosStatus, BootInterfaceSelector, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::core::{ActionError, Bmc};

use crate::bios::attributes::BiosAttribute;
use crate::bios::standard::StandardBios;
use crate::bios::support::{RedfishBiosExt as _, compare, expected};
use crate::resources::RedfishResourcesExt as _;

/// NVIDIA GB200 and GB300 NVL compute trays: OpenBMC naming the UEFI
/// administrator password `AdminPassword`, and reporting no BIOS attributes
/// after a BMC restart until the host starts again.
pub(crate) struct Gbx00Bios;

const UEFI_PASSWORD_NAME: &str = "AdminPassword";

/// Option ROMs stay enabled so the DPU appears among the host's network
/// devices and boot options.
const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("TPM", "Enabled").required(),
    INFINITE_BOOT,
    BiosAttribute::bool("Socket0Pcie6DisableOptionROM", false),
    BiosAttribute::bool("Socket1Pcie6DisableOptionROM", false),
];

/// Disabling the embedded UEFI shell keeps the BIOS retrying boot instead of
/// dropping into the shell.
const INFINITE_BOOT: BiosAttribute =
    BiosAttribute::string("EmbeddedUefiShell", "Disabled").required();

#[async_trait]
impl<B: Bmc> Bios<B> for Gbx00Bios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = cx.reported_bios_settings().await?;
        cx.stage_bios_attributes(&expected(ATTRIBUTES, &current, profile).attributes)
            .await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
        _boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<BiosStatus, PlatformError> {
        let current = cx.reported_bios_settings().await?;
        Ok(compare(&current, &expected(ATTRIBUTES, &current, profile)))
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        cx.change_bios_password(UEFI_PASSWORD_NAME, current_password, new_password)
            .await
    }

    async fn infinite_boot_enabled(&self, cx: &OpCx<'_, B>) -> Result<Option<bool>, PlatformError> {
        Ok(cx
            .reported_bios_settings()
            .await?
            .attributes
            .get(INFINITE_BOOT.name)
            .map(|value| INFINITE_BOOT.value.matches(value)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::test_support::Fixture;

    const SYSTEM: &str = "/redfish/v1/Systems/System_0";
    const BIOS: &str = "/redfish/v1/Systems/System_0/Bios";

    /// A BMC restarted since the host last started reports an empty `Bios`
    /// resource: applying waits instead of sending settings the BMC rejects,
    /// and the infinite-boot setting is not ready rather than absent.
    #[tokio::test]
    async fn empty_bios_attributes_are_not_ready() {
        for (scenario, attributes, infinite_boot) in [
            ("an empty Bios resource", json!({}), None),
            (
                "reported attributes",
                json!({"EmbeddedUefiShell": "Disabled"}),
                Some(true),
            ),
        ] {
            let bmc = Fixture::new("NVIDIA", "GB BMC", "System_0", "BMC_0")
                .document(
                    SYSTEM,
                    json!({"@odata.id": SYSTEM, "Id": "System_0", "Name": "System",
                           "Bios": {"@odata.id": BIOS}}),
                )
                .document(
                    BIOS,
                    json!({"@odata.id": BIOS, "Id": "BIOS", "Name": "BIOS",
                           "Attributes": attributes}),
                )
                .build()
                .await;
            let cx = bmc.cx().await;

            let enabled = Gbx00Bios.infinite_boot_enabled(&cx).await;
            match infinite_boot {
                Some(expected) => assert_eq!(enabled.ok(), Some(Some(expected)), "{scenario}"),
                None => {
                    assert!(
                        matches!(enabled, Err(PlatformError::NotReady { .. })),
                        "{scenario}: {enabled:?}"
                    );
                    let applied = Gbx00Bios.apply(&cx, &BiosSettings::default(), None).await;
                    assert!(
                        matches!(applied, Err(PlatformError::NotReady { .. })),
                        "{scenario}: {applied:?}"
                    );
                    assert!(bmc.writes().is_empty(), "{scenario}: nothing is sent");
                }
            }
        }
    }
}
