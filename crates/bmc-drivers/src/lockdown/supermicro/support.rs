/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! The host KCS control the Supermicro lockdown drivers share.

use bmc_platform::{DriverOutcome, OpCx, PlatformError, Quirk};
use nv_redfish::core::Bmc;
use nv_redfish::oem::supermicro::kcs_interface::Privilege;

use crate::lockdown::support::{Signal, signal};

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
