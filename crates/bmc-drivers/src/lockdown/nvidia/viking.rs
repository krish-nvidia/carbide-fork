/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Lockdown, LockdownDesiredState, LockdownScope, LockdownStatus, OpCx,
    PlatformError,
};
use nv_redfish::core::Bmc;
use serde_json::{Map, Value};

use crate::lockdown::support::{signal, state_from_signals, status};
use crate::resources::{patch_bios_attributes, selected_bios};

/// NVIDIA Viking lockdown driver; both host and BMC lockdown are BIOS attributes.
///
/// Firmware generations differ in both the KCS attribute name and its value
/// vocabulary, so the write mirrors whichever encoding the BIOS reports.
pub(crate) struct VikingLockdown;

#[async_trait]
impl<B: Bmc> Lockdown<B> for VikingLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let bios = selected_bios(cx).await?;
        let kcs = bios
            .attribute("KcsInterfaceDisable")
            .or_else(|| bios.attribute("IPMIKCSInterfaceDisable"))
            .and_then(|value| value.str_value().map(str::to_owned));
        let redfish = bios
            .attribute("RedfishEnable")
            .and_then(|value| value.str_value().map(str::to_owned));
        let host = state_from_signals(&[(
            matches!(kcs.as_deref(), Some("Deny All" | "Disabled")),
            matches!(kcs.as_deref(), Some("Allow All" | "Enabled")),
        )]);
        let bmc = state_from_signals(&[signal(redfish.as_deref(), "Disabled", "Enabled")]);
        Ok(status(
            host,
            bmc,
            format!("kcs_interface_disable={kcs:?}, redfish_enable={redfish:?}"),
        ))
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        scope: LockdownScope,
        desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        let enabled = desired == LockdownDesiredState::Enabled;
        let bios = selected_bios(cx).await?;
        let mut attributes = Map::new();
        if matches!(scope, LockdownScope::Host | LockdownScope::All) {
            let key = if bios.attribute("KcsInterfaceDisable").is_some() {
                "KcsInterfaceDisable"
            } else {
                "IPMIKCSInterfaceDisable"
            };
            let current = bios
                .attribute(key)
                .and_then(|value| value.str_value().map(str::to_owned));
            let kcs = match (current.as_deref(), enabled) {
                (Some("Enabled" | "Disabled"), true) => "Disabled",
                (Some("Enabled" | "Disabled"), false) => "Enabled",
                (_, true) => "Deny All",
                (_, false) => "Allow All",
            };
            attributes.insert(key.to_string(), kcs.into());
        }
        if matches!(
            scope,
            LockdownScope::Bmc | LockdownScope::BmcSystemLockdown | LockdownScope::All
        ) {
            attributes.insert(
                "RedfishEnable".to_string(),
                (if enabled { "Disabled" } else { "Enabled" }).into(),
            );
        }
        patch_bios_attributes(cx, Value::Object(attributes)).await
    }
}
