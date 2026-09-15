/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Host and BMC lockdown capability drivers.

use bmc_platform::{DriverOutcome, LockdownState, LockdownStatus, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::host_interface::HostInterface;
use serde_json::json;

mod ami;
mod dell;
mod hpe;
mod lenovo;
mod nvidia;
mod supermicro;

pub(crate) use ami::MegaRacLockdown;
pub(crate) use dell::IdracLockdown;
pub(crate) use hpe::IloLockdown;
pub(crate) use lenovo::{Gb300Lockdown, LenovoAmiLockdown, XccLockdown};
pub(crate) use nvidia::{OpenBmcLockdown, VikingLockdown};
pub(crate) use supermicro::{Ars121lLockdown, SmcLockdown};

async fn host_interfaces<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Vec<HostInterface<B>>, PlatformError> {
    cx.manager()?
        .host_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

/// BMC lockdown state derived from the manager's first host interface.
async fn host_interface_state<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<(LockdownState, Option<bool>), PlatformError> {
    let enabled = host_interfaces(cx)
        .await?
        .first()
        .and_then(|interface| interface.interface_enabled());
    Ok((state_from_signals(&[signal(enabled, false, true)]), enabled))
}

async fn set_first_host_interface<B: Bmc>(
    cx: &OpCx<'_, B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let interface = host_interfaces(cx)
        .await?
        .into_iter()
        .next()
        .ok_or(PlatformError::NoContent)?;
    let raw = interface.raw();
    cx.patch(raw.as_ref(), &json!({"InterfaceEnabled": enabled}))
        .await
}

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
