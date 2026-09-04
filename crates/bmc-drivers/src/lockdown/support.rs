/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    DriverOutcome, Lockdown, LockdownDesiredState, LockdownScope, LockdownState, LockdownStatus,
    OpCx, PlatformError,
};
use nv_redfish::core::Bmc;
use serde_json::{Map, Value};

use crate::lockdown::{host_interface_state, set_first_host_interface};
use crate::resources::{patch_bios_attributes, selected_bios};

/// One control's observation: `(locked, unlocked)`, both false when unknown.
pub(super) type Signal = (bool, bool);

/// Reads a control whose two known values mean locked and unlocked.
pub(super) fn signal<T: PartialEq>(actual: Option<T>, locked: T, unlocked: T) -> Signal {
    (actual == Some(locked), actual == Some(unlocked))
}

/// Aggregates `(locked, unlocked)` observations of independent controls.
pub(super) fn state_from_signals(signals: &[Signal]) -> LockdownState {
    if signals
        .iter()
        .all(|(locked, unlocked)| !locked && !unlocked)
    {
        return LockdownState::Unknown;
    }
    if signals.iter().all(|(locked, _)| *locked) {
        LockdownState::Enabled
    } else if signals.iter().all(|(_, unlocked)| *unlocked) {
        LockdownState::Disabled
    } else {
        LockdownState::Partial
    }
}

pub(super) fn status(host: LockdownState, bmc: LockdownState, message: String) -> LockdownStatus {
    let aggregate = match (host, bmc) {
        (LockdownState::Unknown, LockdownState::Unknown) => LockdownState::Unknown,
        (LockdownState::Enabled, LockdownState::Enabled) => LockdownState::Enabled,
        (LockdownState::Disabled, LockdownState::Disabled) => LockdownState::Disabled,
        _ => LockdownState::Partial,
    };
    LockdownStatus {
        aggregate,
        message,
        host,
        bmc,
    }
}

/// A BIOS attribute whose two values mean locked and unlocked.
pub(crate) struct LockedAttribute {
    pub(crate) key: &'static str,
    pub(crate) locked: &'static str,
    pub(crate) unlocked: &'static str,
}

/// Lockdown for BMCs whose host controls are BIOS attributes and whose BMC
/// control is the manager's host interface.
pub(crate) struct BiosAttributeLockdown {
    pub(crate) host: &'static [LockedAttribute],
}

#[async_trait]
impl<B: Bmc> Lockdown<B> for BiosAttributeLockdown {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<LockdownStatus, PlatformError> {
        let bios = selected_bios(cx).await?;
        let values: Vec<(&str, Option<String>)> = self
            .host
            .iter()
            .map(|attribute| {
                let value = bios
                    .attribute(attribute.key)
                    .and_then(|value| value.str_value().map(str::to_owned));
                (attribute.key, value)
            })
            .collect();
        let signals: Vec<_> = self
            .host
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
            let attributes: Map<String, Value> = self
                .host
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_and_component_aggregation_preserves_partial() {
        assert_eq!(state_from_signals(&[]), LockdownState::Unknown);
        assert_eq!(
            state_from_signals(&[(false, false), (false, false)]),
            LockdownState::Unknown
        );
        assert_eq!(
            state_from_signals(&[(true, false), (true, false)]),
            LockdownState::Enabled
        );
        assert_eq!(
            state_from_signals(&[(false, true), (false, true)]),
            LockdownState::Disabled
        );
        assert_eq!(
            state_from_signals(&[(true, false), (false, true)]),
            LockdownState::Partial
        );
        assert_eq!(
            status(
                LockdownState::Enabled,
                LockdownState::Disabled,
                String::new()
            )
            .aggregate,
            LockdownState::Partial
        );
    }
}
