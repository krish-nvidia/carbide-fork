/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! The KCS, host-interface, and system-lockdown controls the Supermicro
//! lockdown drivers share.

use bmc_platform::{DriverOutcome, LockdownScope, OpCx, PlatformError, Quirk};
use nv_redfish::core::Bmc;
use nv_redfish::oem::supermicro::kcs_interface::Privilege;

use crate::lockdown::support::{RedfishLockdownExt as _, Signal, signal};

/// Whether `scope` covers the OEM `SysLockdown` switch; Supermicro has no
/// separate [`LockdownScope::BmcSystemLockdown`].
pub(super) fn sys_lockdown_scope(scope: LockdownScope) -> Result<bool, PlatformError> {
    match scope {
        LockdownScope::Bmc | LockdownScope::All => Ok(true),
        LockdownScope::Host => Ok(false),
        LockdownScope::BmcSystemLockdown => Err(PlatformError::Unsupported),
    }
}

/// A KCS state the BMC does not report counts as both locked and unlocked.
pub(super) fn kcs_signal(privilege: Option<Privilege>) -> Signal {
    privilege.map_or((true, true), |privilege| {
        signal(
            Some(privilege),
            Privilege::Callback,
            Privilege::Administrator,
        )
    })
}

/// Whether the OEM `SysLockdown` switch is on; `None` when unreported.
pub(super) async fn sys_lockdown<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Option<bool>, PlatformError> {
    Ok(cx
        .manager()
        .await?
        .oem_supermicro()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .sys_lockdown()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .and_then(|value| value.sys_lockdown_enabled()))
}

pub(super) async fn set_sys_lockdown<B: Bmc>(
    cx: &OpCx<'_, B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    cx.manager()
        .await?
        .oem_supermicro()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .sys_lockdown()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .set_enabled(enabled)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Whether the first host interface is enabled; `None` when unreported.
pub(super) async fn host_interface_enabled<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<bool>, PlatformError> {
    Ok(cx
        .host_interfaces()
        .await?
        .first()
        .and_then(|interface| interface.interface_enabled()))
}

/// Enables or disables every host interface.
pub(super) async fn set_host_interfaces<B: Bmc>(
    cx: &OpCx<'_, B>,
    enabled: bool,
) -> Result<DriverOutcome, PlatformError> {
    let mut outcome = DriverOutcome::complete();
    for interface in cx.host_interfaces().await? {
        outcome = outcome.merge(cx.set_host_interface(&interface, enabled).await?);
    }
    Ok(outcome)
}

/// Host KCS access: the OEM `KCSInterface` privilege, or on MGX C2 the system
/// `IPMIHostInterface` read as `Administrator` when enabled and `Callback`
/// when disabled. `None` on MGX C2 firmware that does not expose it.
pub(super) async fn kcs_privilege<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<Option<Privilege>, PlatformError> {
    if cx.has_quirk(Quirk::SupermicroMgxC2) {
        if !cx.has_quirk(Quirk::SupermicroIpmiHostInterface) {
            return Ok(None);
        }
        let enabled = cx
            .system()
            .await?
            .ipmi_host_interface_enabled()
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: "MGX C2 system does not report IPMIHostInterface".to_string(),
            })?;
        return Ok(Some(if enabled {
            Privilege::Administrator
        } else {
            Privilege::Callback
        }));
    }
    cx.manager()
        .await?
        .oem_supermicro()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .kcs_interface()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .privilege()
        .map(Some)
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: "Supermicro KCSInterface does not report Privilege".to_string(),
        })
}

/// Sets host KCS access; on MGX C2 this is the system `IPMIHostInterface`,
/// left unchanged on firmware that does not expose it.
pub(super) async fn set_kcs_privilege<B: Bmc>(
    cx: &OpCx<'_, B>,
    privilege: Privilege,
) -> Result<DriverOutcome, PlatformError> {
    if cx.has_quirk(Quirk::SupermicroMgxC2) {
        if !cx.has_quirk(Quirk::SupermicroIpmiHostInterface) {
            return Ok(DriverOutcome::complete());
        }
        return cx
            .system()
            .await?
            .set_ipmi_host_interface_enabled(privilege == Privilege::Administrator)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error));
    }
    cx.manager()
        .await?
        .oem_supermicro()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .kcs_interface()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .set_privilege(privilege)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bmc_platform::{Lockdown, LockdownDesiredState, LockdownScope, Quirk};
    use serde_json::json;

    use crate::lockdown::supermicro::smc::SmcLockdown;
    use crate::test_support::{Fixture, body, path};

    const MANAGER: &str = "/redfish/v1/Managers/1";
    const HOST_INTERFACES: &str = "/redfish/v1/Managers/1/HostInterfaces";

    #[tokio::test]
    async fn mgx_c2_host_kcs_is_the_system_ipmi_host_interface_on_supporting_firmware() {
        let cases = [
            (
                "firmware exposing IPMIHostInterface",
                BTreeSet::from([Quirk::SupermicroMgxC2, Quirk::SupermicroIpmiHostInterface]),
                vec![json!({"IPMIHostInterface": {"ServiceEnabled": false}})],
            ),
            (
                "older firmware",
                BTreeSet::from([Quirk::SupermicroMgxC2]),
                vec![],
            ),
        ];
        for (scenario, quirks, expected) in cases {
            let bmc = Fixture::new("Supermicro", "", "1", "1")
                .document(
                    MANAGER,
                    json!({
                        "@odata.id": MANAGER,
                        "Id": "1",
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
                        "Members": [],
                    }),
                )
                .build()
                .await;
            let cx = bmc.cx().await.with_quirks(&quirks);

            SmcLockdown
                .set(&cx, LockdownScope::Host, LockdownDesiredState::Enabled)
                .await
                .expect("host lockdown succeeds");

            let writes = bmc.writes();
            assert!(
                writes
                    .iter()
                    .all(|request| path(request) == "/redfish/v1/Systems/1"),
                "{scenario}"
            );
            assert_eq!(
                writes.iter().map(body).collect::<Vec<_>>(),
                expected,
                "{scenario}"
            );
        }
    }
}
