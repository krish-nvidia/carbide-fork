/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BlueField drivers against documents trimmed from recorded BF3 and BF4 BMCs.

use axum::http::header::IF_MATCH;
use axum::http::{Method, StatusCode};
use bmc_mock::test_support::TestBmc;
use bmc_platform::{
    ControllerAction, Dpu, DpuStatus, DriverOutcome, HostPrivilegeLevel, NicMode, PlatformError,
    RshimState,
};
use serde_json::{Value, json};

use super::bluefield3::BlueField3Dpu;
use super::bluefield4::BlueField4Dpu;
use crate::test_support::{Fixture, body, path};

const BF3_SYSTEM: &str = "/redfish/v1/Systems/Bluefield";
const BF3_SYSTEM_OEM: &str = "/redfish/v1/Systems/Bluefield/Oem/Nvidia";
const BF3_BIOS: &str = "/redfish/v1/Systems/Bluefield/Bios";
const BF3_BIOS_SETTINGS: &str = "/redfish/v1/Systems/Bluefield/Bios/Settings";
const BF3_MANAGER: &str = "/redfish/v1/Managers/Bluefield_BMC";
const BF3_MANAGER_OEM: &str = "/redfish/v1/Managers/Bluefield_BMC/Oem/Nvidia";
const FIRMWARE_INVENTORY: &str = "/redfish/v1/UpdateService/FirmwareInventory";

const BF4_ADAPTER: &str = "/redfish/v1/Chassis/BlueField_0/NetworkAdapters/BlueField_NIC_0";
const BF4_ADAPTER_SETTINGS: &str =
    "/redfish/v1/Chassis/BlueField_0/NetworkAdapters/BlueField_NIC_0/Settings";
const BF4_PRIVILEGES: &str = "/redfish/v1/Chassis/BlueField_0/NetworkAdapters/BlueField_NIC_0/Oem/Nvidia/HostPrivilegeConfig";
const BF4_PRIVILEGES_SETTINGS: &str = "/redfish/v1/Chassis/BlueField_0/NetworkAdapters/BlueField_NIC_0/Oem/Nvidia/HostPrivilegeConfig/Settings";

fn bluefield3(bmc_firmware: &str) -> Fixture {
    Fixture::new("Nvidia", "BlueField-3 DPU", "Bluefield", "Bluefield_BMC")
        .document(
            BF3_SYSTEM,
            json!({
                "@odata.id": BF3_SYSTEM,
                "Id": "Bluefield",
                "Name": "Bluefield",
                "Bios": {"@odata.id": BF3_BIOS},
                "Oem": {"Nvidia": {"@odata.id": BF3_SYSTEM_OEM}},
            }),
        )
        .document(
            BF3_SYSTEM_OEM,
            json!({
                "@odata.id": BF3_SYSTEM_OEM,
                "@odata.type": "#NvidiaComputerSystem.v1_0_0.NvidiaComputerSystem",
                "Actions": {
                    "#HostRshim.Set": {"target": format!("{BF3_SYSTEM_OEM}/Actions/HostRshim.Set")},
                    "#Mode.Set": {"target": format!("{BF3_SYSTEM_OEM}/Actions/Mode.Set")},
                },
                "BaseMAC": "3825f33aabc4",
                "HostRshim": "Disabled",
                "Mode": "DpuMode",
            }),
        )
        .document(
            BF3_MANAGER,
            json!({
                "@odata.id": BF3_MANAGER,
                "Id": "Bluefield_BMC",
                "Name": "OpenBmc Manager",
                "Oem": {"Nvidia": {
                    "@odata.id": BF3_MANAGER_OEM,
                    "@odata.type": "#NvidiaManager.v1_6_0.NvidiaManager",
                }},
            }),
        )
        .document(
            "/redfish/v1/UpdateService",
            json!({
                "@odata.id": "/redfish/v1/UpdateService",
                "Id": "UpdateService",
                "Name": "Update Service",
                "FirmwareInventory": {"@odata.id": FIRMWARE_INVENTORY},
            }),
        )
        .document(
            FIRMWARE_INVENTORY,
            json!({
                "@odata.id": FIRMWARE_INVENTORY,
                "@odata.type": "#SoftwareInventoryCollection.SoftwareInventoryCollection",
                "Name": "Software Inventory Collection",
                "Members": [{"@odata.id": format!("{FIRMWARE_INVENTORY}/BMC_Firmware")}],
            }),
        )
        .document(
            &format!("{FIRMWARE_INVENTORY}/BMC_Firmware"),
            json!({
                "@odata.id": format!("{FIRMWARE_INVENTORY}/BMC_Firmware"),
                "Id": "BMC_Firmware",
                "Name": "Software Inventory",
                "Version": bmc_firmware,
            }),
        )
        .document(
            BF3_BIOS,
            json!({
                "@odata.id": BF3_BIOS,
                "@Redfish.Settings": {"SettingsObject": {"@odata.id": BF3_BIOS_SETTINGS}},
                "Id": "BIOS",
                "Name": "BIOS Configuration Current Settings",
                "Attributes": {
                    "HostPrivilegeLevel": "Privileged",
                    "InternalCPUModel": "Embedded",
                    "NicMode": "DpuMode",
                },
            }),
        )
        .document(
            BF3_BIOS_SETTINGS,
            json!({
                "@odata.id": BF3_BIOS_SETTINGS,
                "Id": "BIOS_Settings",
                "Name": "BIOS Configuration",
                "Attributes": {},
            }),
        )
}

fn bluefield4() -> Fixture {
    Fixture::new("Nvidia", "BlueField-4", "BlueField_0", "BlueField_BMC_0")
        .document(
            "/redfish/v1/Chassis",
            json!({
                "@odata.id": "/redfish/v1/Chassis",
                "@odata.type": "#ChassisCollection.ChassisCollection",
                "Name": "Chassis Collection",
                "Members": [{"@odata.id": "/redfish/v1/Chassis/BlueField_0"}],
            }),
        )
        .document(
            "/redfish/v1/Chassis/BlueField_0",
            json!({
                "@odata.id": "/redfish/v1/Chassis/BlueField_0",
                "Id": "BlueField_0",
                "Name": "BlueField_0",
                "ChassisType": "Card",
                "Model": "B4240V",
                "NetworkAdapters": {"@odata.id": "/redfish/v1/Chassis/BlueField_0/NetworkAdapters"},
            }),
        )
        .document(
            "/redfish/v1/Chassis/BlueField_0/NetworkAdapters",
            json!({
                "@odata.id": "/redfish/v1/Chassis/BlueField_0/NetworkAdapters",
                "@odata.type": "#NetworkAdapterCollection.NetworkAdapterCollection",
                "Name": "Network Adapter Collection",
                "Members": [{"@odata.id": BF4_ADAPTER}],
            }),
        )
        .document(
            BF4_ADAPTER,
            json!({
                "@odata.id": BF4_ADAPTER,
                "@odata.etag": "\"adapter-1\"",
                "@Redfish.Settings": {"SettingsObject": {"@odata.id": BF4_ADAPTER_SETTINGS}},
                "Id": "BlueField_NIC_0",
                "Name": "Network Adapter",
                "Oem": {"Nvidia": {
                    "@odata.type": "#NvidiaNetworkAdapter.v1_2_0.NvidiaNetworkAdapter",
                    "BaseMAC": "f4:20:4d:14:aa:e2",
                    "DPUOperationMode": "DPU",
                    "HostPrivilegeConfig": {"@odata.id": BF4_PRIVILEGES},
                }},
            }),
        )
        .document(
            BF4_ADAPTER_SETTINGS,
            json!({
                "@odata.id": BF4_ADAPTER_SETTINGS,
                "@odata.etag": "\"adapter-settings-1\"",
                "Id": "Settings",
                "Name": "Network Adapter Settings",
                "Oem": {"Nvidia": {"DPUOperationMode": "DPU"}},
            }),
        )
        .document(
            BF4_PRIVILEGES,
            json!({
                "@odata.id": BF4_PRIVILEGES,
                "@Redfish.Settings": {"SettingsObject": {"@odata.id": BF4_PRIVILEGES_SETTINGS}},
                "Id": "HostPrivilegeConfig",
                "Name": "Host Privilege Configuration",
                "PrivilegeMode": "Custom",
                "PrivilegeSettings": {"HostPrivilegeLevel": "Restricted"},
            }),
        )
        .document(
            BF4_PRIVILEGES_SETTINGS,
            json!({
                "@odata.id": BF4_PRIVILEGES_SETTINGS,
                "@odata.etag": "\"privileges-settings-1\"",
                "Id": "Settings",
                "Name": "Host Privilege Configuration Settings",
                "PrivilegeMode": "Custom",
            }),
        )
}

#[tokio::test]
async fn bluefield3_status_reads_the_system_oem_body_on_supported_firmware() {
    for (firmware, nic_mode) in [("BF-26.04-8", Some(NicMode::Dpu)), ("BF-23.09-9", None)] {
        let bmc = bluefield3(firmware).build().await;
        let cx = bmc.cx().await;
        assert_eq!(
            BlueField3Dpu.status(&cx).await,
            Ok(DpuStatus {
                nic_mode,
                host_rshim: Some(RshimState::Disabled),
            }),
            "{firmware}"
        );
    }
}

#[tokio::test]
async fn bluefield3_avoids_the_oem_resource_on_firmware_where_it_times_out() {
    let nic_mode_bios_error = json!({"Attributes": {"NicMode": "NicMode"}});
    let bmc = bluefield3("BF-24.04-5")
        .respond(
            Method::GET,
            BF3_SYSTEM_OEM,
            StatusCode::GATEWAY_TIMEOUT,
            None,
        )
        .respond(
            Method::GET,
            BF3_BIOS,
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(nic_mode_bios_error),
        )
        .build()
        .await;
    let cx = bmc.cx().await;

    assert_eq!(
        BlueField3Dpu.status(&cx).await,
        Ok(DpuStatus {
            nic_mode: Some(NicMode::Nic),
            host_rshim: None,
        })
    );
    assert_eq!(
        BlueField3Dpu.set_nic_mode(&cx, NicMode::Dpu).await,
        Ok(DriverOutcome::complete())
    );
    let writes = bmc.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        path(&writes[0]),
        format!("{BF3_SYSTEM_OEM}/Actions/Mode.Set")
    );
    assert_eq!(body(&writes[0]), json!({"Mode": "DpuMode"}));
}

#[tokio::test]
async fn bluefield3_host_privilege_retries_with_the_spaced_attribute_name() {
    let rejection = json!({"error": {"@Message.ExtendedInfo": [{
        "MessageId": "Base.1.15.PropertyUnknown",
        "Message": "The property HostPrivilegeLevel is not in the list of valid properties for the resource.",
    }]}});
    let unspaced = json!({"Attributes": {"HostPrivilegeLevel": "Restricted"}});
    let spaced = json!({"Attributes": {"Host Privilege Level": "Restricted"}});

    for (rejects_unspaced, expected_bodies) in [
        (false, vec![unspaced.clone()]),
        (true, vec![unspaced.clone(), spaced.clone()]),
    ] {
        let mut fixture = bluefield3("BF-24.07-14");
        if rejects_unspaced {
            fixture = fixture.respond(
                Method::PATCH,
                BF3_BIOS_SETTINGS,
                StatusCode::BAD_REQUEST,
                Some(rejection.clone()),
            );
        }
        let bmc = fixture.build().await;
        let cx = bmc.cx().await;

        let result = BlueField3Dpu
            .set_host_privilege_level(&cx, HostPrivilegeLevel::Restricted)
            .await;
        assert_eq!(result.is_ok(), !rejects_unspaced, "{rejects_unspaced}");
        let writes = bmc.writes();
        assert!(
            writes.iter().all(|write| path(write) == BF3_BIOS_SETTINGS),
            "{rejects_unspaced}"
        );
        assert_eq!(
            writes.iter().map(body).collect::<Vec<_>>(),
            expected_bodies,
            "{rejects_unspaced}"
        );
    }
}

#[tokio::test]
async fn nic_mode_waits_for_an_operator_while_host_privilege_is_restricted() {
    let restricted_bios = json!({
        "@odata.id": BF3_BIOS,
        "@Redfish.Settings": {"SettingsObject": {"@odata.id": BF3_BIOS_SETTINGS}},
        "Id": "BIOS",
        "Name": "BIOS Configuration Current Settings",
        "Attributes": {"HostPrivilegeLevel": "Restricted", "NicMode": "DpuMode"},
    });
    let blocked = DriverOutcome::blocked(ControllerAction::ManualIntervention {
        code: "dpu-host-privilege-restricted".parse().expect("valid code"),
    });
    struct Case {
        name: &'static str,
        fixture: Fixture,
        dpu: &'static dyn Dpu<TestBmc>,
        expected: DriverOutcome,
        writes: Vec<Value>,
    }
    let cases = [
        Case {
            name: "BF3 restricted",
            fixture: bluefield3("BF-26.04-8").document(BF3_BIOS, restricted_bios),
            dpu: &BlueField3Dpu,
            expected: blocked.clone(),
            writes: vec![],
        },
        Case {
            name: "BF3 privileged",
            fixture: bluefield3("BF-26.04-8"),
            dpu: &BlueField3Dpu,
            expected: DriverOutcome::complete(),
            writes: vec![json!({"Mode": "NicMode"})],
        },
        Case {
            name: "BF4 restricted",
            fixture: bluefield4(),
            dpu: &BlueField4Dpu,
            expected: blocked,
            writes: vec![],
        },
    ];

    for Case {
        name,
        fixture,
        dpu,
        expected,
        writes,
    } in cases
    {
        let bmc = fixture.build().await;
        let cx = bmc.cx().await;
        assert_eq!(
            dpu.set_nic_mode(&cx, NicMode::Nic).await,
            Ok(expected),
            "{name}"
        );
        assert_eq!(
            bmc.writes().iter().map(body).collect::<Vec<_>>(),
            writes,
            "{name}"
        );
    }
}

#[tokio::test]
async fn bmc_rshim_is_enabled_through_the_manager_resource_the_dpu_links() {
    let bmc = bluefield3("BF-26.04-8").build().await;
    let cx = bmc.cx().await;
    assert_eq!(
        BlueField3Dpu.enable_bmc_rshim(&cx).await,
        Ok(DriverOutcome::complete())
    );
    let writes = bmc.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(path(&writes[0]), BF3_MANAGER_OEM);
    assert_eq!(writes[0].headers[IF_MATCH], "*");
    assert_eq!(
        body(&writes[0]),
        json!({"BmcRShim": {"BmcRShimEnabled": true}})
    );

    let bmc = bluefield4().build().await;
    let cx = bmc.cx().await;
    assert_eq!(
        BlueField4Dpu.enable_bmc_rshim(&cx).await,
        Err(PlatformError::Unsupported)
    );
    assert!(bmc.writes().is_empty());
}

#[tokio::test]
async fn bluefield4_status_reads_the_network_adapter() {
    let bmc = bluefield4().build().await;
    let cx = bmc.cx().await;
    assert_eq!(
        BlueField4Dpu.status(&cx).await,
        Ok(DpuStatus {
            nic_mode: Some(NicMode::Dpu),
            host_rshim: None,
        })
    );
}

#[tokio::test]
async fn bluefield4_mode_and_privileges_are_written_to_their_settings_objects() {
    let bmc = bluefield4().build().await;
    let cx = bmc.cx().await;
    BlueField4Dpu
        .set_nic_mode(&cx, NicMode::Dpu)
        .await
        .expect("mode update succeeds");
    BlueField4Dpu
        .set_host_privilege_level(&cx, HostPrivilegeLevel::Restricted)
        .await
        .expect("privilege update succeeds");

    let writes = bmc.writes();
    assert_eq!(
        writes
            .iter()
            .map(|write| (
                path(write),
                write.headers[IF_MATCH].to_str().unwrap(),
                body(write)
            ))
            .collect::<Vec<_>>(),
        [
            (
                BF4_ADAPTER_SETTINGS,
                "\"adapter-settings-1\"",
                json!({"Oem": {"Nvidia": {"DPUOperationMode": "DPU"}}}),
            ),
            (
                BF4_PRIVILEGES_SETTINGS,
                "\"privileges-settings-1\"",
                json!({"PrivilegeMode": "Restricted"}),
            ),
        ]
    );
}
