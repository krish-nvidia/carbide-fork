/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Lockdown signal aggregation and host-interface controls shared by several
//! vendor drivers.

use bmc_platform::{DriverOutcome, LockdownState, LockdownStatus, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::host_interface::{HostInterface, HostInterfaceUpdate};

pub(super) async fn host_interfaces<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Vec<HostInterface<B>>, PlatformError> {
    cx.manager()
        .await?
        .host_interfaces()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))
}

/// BMC lockdown state derived from the manager's first host interface, which
/// is enabled unless it reports otherwise.
pub(super) async fn host_interface_state<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<(LockdownState, Option<bool>), PlatformError> {
    let enabled = host_interfaces(cx)
        .await?
        .first()
        .map(|interface| interface.interface_enabled().unwrap_or(true));
    Ok((state_from_signals(&[signal(enabled, false, true)]), enabled))
}

pub(super) async fn set_first_host_interface<B: Bmc>(
    cx: &OpCx<'_, B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let interface = host_interfaces(cx)
        .await?
        .into_iter()
        .next()
        .ok_or(PlatformError::NoContent)?;
    set_host_interface(cx, &interface, enabled).await
}

/// Enables or disables one host interface.
pub(super) async fn set_host_interface<B: Bmc>(
    cx: &OpCx<'_, B>,
    interface: &HostInterface<B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let body = HostInterfaceUpdate::builder()
        .with_interface_enabled(enabled)
        .build();
    interface
        .update(&body)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
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
    use serde_json::json;

    use super::*;
    use crate::test_support::Fixture;

    #[tokio::test]
    async fn a_host_interface_that_omits_interface_enabled_counts_as_enabled() {
        const MANAGER: &str = "/redfish/v1/Managers/Self";
        const HOST_INTERFACES: &str = "/redfish/v1/Managers/Self/HostInterfaces";
        const HOST_INTERFACE: &str = "/redfish/v1/Managers/Self/HostInterfaces/Self";
        let bmc = Fixture::new("AMI", "AMI Redfish Server", "Self", "Self")
            .document(
                MANAGER,
                json!({
                    "@odata.id": MANAGER,
                    "Id": "Self",
                    "Name": "Manager",
                    "HostInterfaces": {"@odata.id": HOST_INTERFACES},
                }),
            )
            .document(
                HOST_INTERFACES,
                json!({
                    "@odata.id": HOST_INTERFACES,
                    "@odata.type": "#HostInterfaceCollection.HostInterfaceCollection",
                    "Name": "Host Interface Collection",
                    "Members": [{"@odata.id": HOST_INTERFACE}],
                }),
            )
            .document(
                HOST_INTERFACE,
                json!({"@odata.id": HOST_INTERFACE, "Id": "Self", "Name": "Host Interface"}),
            )
            .build()
            .await;
        let cx = bmc.cx().await;

        assert_eq!(
            host_interface_state(&cx).await,
            Ok((LockdownState::Disabled, Some(true)))
        );
    }

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
