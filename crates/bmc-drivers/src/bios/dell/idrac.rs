/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, BiosSettings, BiosStatus, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::{Value, json};

use crate::bios::attributes::{BiosAttribute, desired_settings};
use crate::bios::standard::StandardBios;
use crate::bios::support::{
    attribute_holds, change_password, compare, current_settings, settings, with_profile,
};
use crate::dell;

/// Dell iDRAC BIOS behavior.
///
/// iDRAC applies staged settings only through a configuration job, refuses
/// new jobs while any is queued, and clears pending settings by deleting the
/// job queue rather than by re-writing attributes. Each write therefore
/// replaces the job of any earlier write not yet applied by a reset.
pub(crate) struct IdracBios;

/// iDRAC exposes the UEFI administrator password as `SetupPassword`.
const UEFI_PASSWORD_NAME: &str = "SetupPassword";

const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("InBandManageabilityInterface", "Disabled").required(),
    BiosAttribute::string("UefiVariableAccess", "Standard").required(),
    BiosAttribute::string("FailSafeBaud", "115200").required(),
    BiosAttribute::string("ConTermType", "Vt100Vt220").required(),
    BiosAttribute::string("RedirAfterBoot", "Enabled"),
    BiosAttribute::string("SriovGlobalEnable", "Enabled").required(),
    BiosAttribute::string("TpmSecurity", "On").required(),
    BiosAttribute::string("Tpm2Hierarchy", "Enabled").required(),
    BiosAttribute::string("Tpm2Algorithm", "SHA256").required(),
    BiosAttribute::string("HttpDev1EnDis", "Enabled").required(),
    BiosAttribute::string("HttpDev1TlsMode", "None").required(),
    BiosAttribute::string("PxeDev1EnDis", "Disabled").required(),
    INFINITE_BOOT,
    // Read-only and already `Uefi` on iDRAC 10, which leaves nothing to write.
    BiosAttribute::string("BootMode", "Uefi"),
];

const INFINITE_BOOT: BiosAttribute = BiosAttribute::string("BootSeqRetry", "Enabled").required();

/// The hierarchy reads back `Enabled` once the clear has run.
fn tpm_clear() -> BiosSettings {
    settings([
        ("TpmSecurity", json!("On")),
        ("Tpm2Hierarchy", json!("Clear")),
    ])
}

/// Serial redirection in the format the BIOS uses: a `SerialPortAddress`
/// starting with `Serial1` marks the newer BIOS, which redirects
/// automatically on COM2.
fn serial_redirection(current: &BiosSettings) -> [BiosAttribute; 2] {
    let newer = current
        .attributes
        .get("SerialPortAddress")
        .and_then(Value::as_str)
        .is_some_and(|address| address.starts_with("Serial1"));
    if newer {
        [
            BiosAttribute::string("SerialComm", "OnConRedirAuto").required(),
            BiosAttribute::string("SerialPortAddress", "Serial1Com2Serial2Com1").required(),
        ]
    } else {
        [
            BiosAttribute::string("SerialComm", "OnConRedir").required(),
            BiosAttribute::string("SerialPortAddress", "Com1").required(),
        ]
    }
}

fn expected_settings(current: &BiosSettings, profile: &BiosSettings) -> BiosSettings {
    let attributes: Vec<BiosAttribute> = ATTRIBUTES
        .iter()
        .copied()
        .chain(serial_redirection(current))
        .collect();
    with_profile(desired_settings(&attributes, current), profile)
}

/// Changes the UEFI password inside a configuration job, which iDRAC requires
/// for the change to take effect.
async fn change_password_with_job<B: Bmc>(
    cx: &OpCx<'_, B>,
    current_password: &str,
    new_password: &str,
) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    dell::clear_job_queue(cx).await?;
    change_password(cx, UEFI_PASSWORD_NAME, current_password, new_password).await?;
    dell::create_bios_config_job(cx).await
}

#[async_trait]
impl<B: Bmc> Bios<B> for IdracBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    /// Stages every setting as one configuration job.
    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        let current = current_settings(cx).await?;
        let writes = expected_settings(&current, profile);
        dell::stage_bios_attributes(cx, &writes.attributes).await
    }

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        profile: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError> {
        let current = current_settings(cx).await?;
        Ok(compare(&current, &expected_settings(&current, profile)))
    }

    async fn clear_pending(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        dell::clear_job_queue(cx).await?;
        Ok(DriverOutcome::complete())
    }

    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        change_password_with_job(cx, current_password, new_password).await
    }

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        match change_password_with_job(cx, current_password, "").await {
            Ok(outcome) => Ok(outcome),
            Err(_) => dell::clear_uefi_password_via_import(cx, current_password).await,
        }
    }

    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        dell::stage_bios_attributes(cx, &tpm_clear().attributes).await
    }

    async fn infinite_boot_enabled(&self, cx: &OpCx<'_, B>) -> Result<Option<bool>, PlatformError> {
        attribute_holds(cx, INFINITE_BOOT).await
    }
}

#[cfg(test)]
mod tests {
    use carbide_test_support::value_scenarios;

    use super::*;
    use crate::bios::attributes::AttributeValue;

    #[test]
    fn serial_redirection_follows_the_port_address_format() {
        value_scenarios!(run = |address: &str| serial_redirection(
            &serde_json::from_value(json!({"attributes": {"SerialPortAddress": address}}))
                .expect("settings")
        )
        .map(|attribute| attribute.value);
            "newer BIOS" {
                "Serial1Com1Serial2Com2" => [
                    AttributeValue::String("OnConRedirAuto"),
                    AttributeValue::String("Serial1Com2Serial2Com1"),
                ],
            }
            "older BIOS" {
                "Com2" => [AttributeValue::String("OnConRedir"), AttributeValue::String("Com1")],
            }
        );
    }
}
