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

use crate::lockdown::{
    host_interface_state, set_first_host_interface, signal, state_from_signals, status,
};
use crate::resources::{patch_bios_attributes, selected_bios};

/// A BIOS attribute whose two values mean locked and unlocked.
struct LockedAttribute {
    key: &'static str,
    locked: &'static str,
    unlocked: &'static str,
}

/// Generic AMI MegaRAC: host lockdown is KCS access and USB support in BIOS;
/// BMC lockdown is the manager's host interface.
pub(crate) struct MegaRacLockdown;

const HOST: &[LockedAttribute] = &[
    LockedAttribute {
        key: "KCSACP",
        locked: "Deny All",
        unlocked: "Allow All",
    },
    LockedAttribute {
        key: "USB000",
        locked: "Disabled",
        unlocked: "Enabled",
    },
];

#[async_trait]
impl<B: Bmc> Lockdown<B> for MegaRacLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let bios = selected_bios(cx).await?;
        let values: Vec<(&str, Option<String>)> = HOST
            .iter()
            .map(|attribute| {
                let value = bios
                    .attribute(attribute.key)
                    .and_then(|value| value.str_value().map(str::to_owned));
                (attribute.key, value)
            })
            .collect();
        let signals: Vec<_> = HOST
            .iter()
            .zip(&values)
            .map(|(attribute, (_, value))| {
                signal(value.as_deref(), attribute.locked, attribute.unlocked)
            })
            .collect();
        let (bmc, interface) = host_interface_state(cx).await?;
        let mut message: Vec<String> = values
            .iter()
            .map(|(key, value)| format!("{key}={value:?}"))
            .collect();
        message.push(format!("host_interface={interface:?}"));
        Ok(status(
            state_from_signals(&signals),
            bmc,
            message.join(", "),
        ))
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        scope: LockdownScope,
        desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        let enabled = desired == LockdownDesiredState::Enabled;
        let mut outcome = DriverOutcome::complete();
        if matches!(scope, LockdownScope::Host | LockdownScope::All) {
            let attributes: Map<String, Value> = HOST
                .iter()
                .map(|attribute| {
                    let value = if enabled {
                        attribute.locked
                    } else {
                        attribute.unlocked
                    };
                    (attribute.key.to_string(), Value::from(value))
                })
                .collect();
            outcome = outcome.merge(patch_bios_attributes(cx, Value::Object(attributes)).await?);
        }
        if matches!(
            scope,
            LockdownScope::Bmc | LockdownScope::BmcSystemLockdown | LockdownScope::All
        ) {
            outcome = outcome.merge(set_first_host_interface(cx, !enabled).await?);
        }
        Ok(outcome)
    }
}
