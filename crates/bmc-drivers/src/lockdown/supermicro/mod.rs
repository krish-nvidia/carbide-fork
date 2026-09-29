/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Supermicro lockdown drivers and the host KCS control they share.

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::Bmc;
use nv_redfish::oem::supermicro::kcs_interface::Privilege;
use version_compare::Cmp;

use crate::lockdown::{Signal, signal};

mod ars121l;
mod smc;

pub(crate) use ars121l::Ars121lLockdown;
pub(crate) use smc::SmcLockdown;

/// The NVIDIA processor module of an MGX C2 system reports this model prefix
/// or part-number fragment.
const MGX_C2_MODEL_PREFIX: &str = "PG535";
const MGX_C2_PART_NUMBER_FRAGMENT: &str = "2G535";
/// MGX C2 BMC firmware exposes the system `IPMIHostInterface` from this version onward.
const MGX_C2_IPMI_HOST_INTERFACE_FIRMWARE: &str = "01.05.01";

/// Whether this is an MGX C2 system, which uses SSIF rather than x86 KCS and
/// so has no `KCSInterface`.
async fn is_mgx_c2<B: Bmc>(cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
    let chassis = cx
        .service_root()
        .chassis()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .members()
        .await
        .map_err(|error| cx.map_redfish_error(error))?;
    Ok(chassis.iter().any(|chassis| {
        let raw = chassis.raw();
        let text = |value: &Option<Option<String>>| value.clone().flatten();
        text(&raw.manufacturer)
            .is_some_and(|manufacturer| manufacturer.eq_ignore_ascii_case("NVIDIA"))
            && (text(&raw.model).is_some_and(|model| model.starts_with(MGX_C2_MODEL_PREFIX))
                || text(&raw.part_number)
                    .is_some_and(|part_number| part_number.contains(MGX_C2_PART_NUMBER_FRAGMENT)))
    }))
}

fn ipmi_host_interface_supported<B: Bmc>(cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
    let firmware = cx
        .manager()?
        .raw()
        .firmware_version
        .clone()
        .flatten()
        .unwrap_or_default();
    Ok(
        version_compare::compare(&firmware, MGX_C2_IPMI_HOST_INTERFACE_FIRMWARE)
            .is_ok_and(|order| order != Cmp::Lt),
    )
}

/// Host KCS access: the OEM `KCSInterface` privilege, or on MGX C2 the system
/// `IPMIHostInterface` read as `Administrator` when enabled and `Callback`
/// when disabled. `None` on MGX C2 firmware that does not report it.
async fn kcs_privilege<B: Bmc>(cx: &OpCx<'_, B>) -> Result<Option<Privilege>, PlatformError> {
    if is_mgx_c2(cx).await? {
        if !ipmi_host_interface_supported(cx)? {
            return Ok(None);
        }
        let enabled = cx.system()?.ipmi_host_interface_enabled().ok_or_else(|| {
            PlatformError::InvalidResponse {
                message: "MGX C2 system does not report IPMIHostInterface".to_string(),
            }
        })?;
        return Ok(Some(if enabled {
            Privilege::Administrator
        } else {
            Privilege::Callback
        }));
    }
    cx.manager()?
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
fn kcs_signal(privilege: Option<Privilege>) -> Signal {
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
async fn set_kcs_privilege<B: Bmc>(
    cx: &OpCx<'_, B>,
    privilege: Privilege,
) -> Result<DriverOutcome, PlatformError> {
    if is_mgx_c2(cx).await? {
        if !ipmi_host_interface_supported(cx)? {
            return Ok(DriverOutcome::complete());
        }
        return cx
            .system()?
            .set_ipmi_host_interface_enabled(privilege == Privilege::Administrator)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error));
    }
    cx.manager()?
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
    use bmc_platform::{EtagMode, Lockdown, LockdownDesiredState, LockdownScope};
    use serde_json::json;

    use super::SmcLockdown;
    use crate::test_support::{Fixture, body, path};

    const MANAGER: &str = "/redfish/v1/Managers/1";
    const HOST_INTERFACES: &str = "/redfish/v1/Managers/1/HostInterfaces";
    const PROCESSOR_MODULE: &str = "/redfish/v1/Chassis/PG535";

    #[tokio::test]
    async fn mgx_c2_host_kcs_is_the_system_ipmi_host_interface_on_supporting_firmware() {
        for (firmware, expected) in [
            (
                "01.05.01",
                vec![json!({"IPMIHostInterface": {"ServiceEnabled": false}})],
            ),
            ("01.04.09", vec![]),
        ] {
            let bmc = Fixture::new("Supermicro", "", "1", "1")
                .document(
                    MANAGER,
                    json!({
                        "@odata.id": MANAGER,
                        "Id": "1",
                        "Name": "Manager",
                        "FirmwareVersion": firmware,
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
                .document(
                    "/redfish/v1/Chassis",
                    json!({
                        "@odata.id": "/redfish/v1/Chassis",
                        "@odata.type": "#ChassisCollection.ChassisCollection",
                        "Name": "Chassis Collection",
                        "Members": [{"@odata.id": PROCESSOR_MODULE}],
                    }),
                )
                .document(
                    PROCESSOR_MODULE,
                    json!({
                        "@odata.id": PROCESSOR_MODULE,
                        "Id": "PG535",
                        "Name": "Processor Module",
                        "ChassisType": "Module",
                        "Manufacturer": "NVIDIA",
                        "Model": "PG535-A00",
                    }),
                )
                .build()
                .await;
            let cx = bmc.cx(EtagMode::Resource).await;

            SmcLockdown
                .set(&cx, LockdownScope::Host, LockdownDesiredState::Enabled)
                .await
                .expect("host lockdown succeeds");

            let writes = bmc.writes();
            assert!(
                writes
                    .iter()
                    .all(|request| path(request) == "/redfish/v1/Systems/1"),
                "{firmware}"
            );
            assert_eq!(
                writes.iter().map(body).collect::<Vec<_>>(),
                expected,
                "{firmware}"
            );
        }
    }
}
