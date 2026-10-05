/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use Capability::{
    Accounts, Attestation, Bios, BmcControl, BootOrder, Console, Dpu, Firmware, Lockdown, Power,
    SecureBoot,
};
use bmc_platform::{
    ChassisIdentity, ManagerIdentity, PlatformIdentity, ServiceRootIdentity, SystemIdentity,
};

use super::*;
use crate::drivers::{Driver, Drivers};
use crate::selection::CapabilitySelection::{self, Standard, Unsupported};
use crate::selection::ResolvedSelection;

fn identity(vendor: &str, product: Option<&str>) -> PlatformIdentity {
    PlatformIdentity {
        service_root: ServiceRootIdentity {
            vendor: Some(vendor.to_string()),
            product: product.map(str::to_string),
            oem_keys: Vec::new(),
        },
        manager: Some(ManagerIdentity {
            id: "BMC".to_string(),
            model: None,
            firmware: None,
        }),
        system: Some(SystemIdentity {
            id: "System".to_string(),
            ..SystemIdentity::default()
        }),
        ..PlatformIdentity::default()
    }
}

fn with_system(mut platform: PlatformIdentity, system: SystemIdentity) -> PlatformIdentity {
    platform.system = Some(system);
    platform
}

fn with_chassis(mut platform: PlatformIdentity, chassis: ChassisIdentity) -> PlatformIdentity {
    platform.chassis = vec![chassis];
    platform
}

fn lenovo_ami() -> PlatformIdentity {
    let mut platform = identity("Lenovo", None);
    platform.service_root.oem_keys = vec!["Ami".to_string()];
    platform
}

fn lenovo_gb300() -> PlatformIdentity {
    with_chassis(
        with_system(
            identity("AMI", Some("AMI Redfish Server")),
            SystemIdentity {
                id: "System_0".to_string(),
                manufacturer: Some("Lenovo".to_string()),
                model: Some("HG634N_V2".to_string()),
                ..SystemIdentity::default()
            },
        ),
        ChassisIdentity {
            id: "HGX_Chassis_0".to_string(),
            model: Some("NVIDIA GB300".to_string()),
            ..ChassisIdentity::default()
        },
    )
}

fn viking() -> PlatformIdentity {
    with_system(
        identity("AMI", Some("AMI Redfish Server")),
        SystemIdentity {
            id: "DGX".to_string(),
            ..SystemIdentity::default()
        },
    )
}

fn dgx_without_bmc_manager() -> PlatformIdentity {
    let mut platform = viking();
    platform.manager = Some(ManagerIdentity {
        id: "Self".to_string(),
        model: None,
        firmware: None,
    });
    platform
}

fn nvswitch() -> PlatformIdentity {
    with_chassis(
        identity("NVIDIA", Some("P3809")),
        ChassisIdentity {
            id: "MGX_NVSwitch_0".to_string(),
            ..ChassisIdentity::default()
        },
    )
}

fn power_shelf(vendor: &str, chassis_id: &str, manufacturer: &str) -> PlatformIdentity {
    with_chassis(
        identity(vendor, None),
        ChassisIdentity {
            id: chassis_id.to_string(),
            manufacturer: Some(manufacturer.to_string()),
            ..ChassisIdentity::default()
        },
    )
}

fn delta() -> PlatformIdentity {
    power_shelf("Delta Electronics Inc.", "chassis", "DELTA")
}

fn liteon() -> PlatformIdentity {
    power_shelf(
        "Lite-On Technology Corp.",
        "powershelf",
        "LITE-ON TECHNOLOGY CORP.",
    )
}

fn supermicro_gb300() -> PlatformIdentity {
    with_system(
        identity("Supermicro", Some("GB NVL")),
        SystemIdentity {
            id: "1".to_string(),
            manufacturer: Some("Supermicro".to_string()),
            model: Some("ARS-121GL-NB2B-GB300".to_string()),
            ..SystemIdentity::default()
        },
    )
}

fn ars121l() -> PlatformIdentity {
    with_system(
        identity("Supermicro", Some("Super Server")),
        SystemIdentity {
            id: "1".to_string(),
            model: Some("ARS-121L-DNR".to_string()),
            ..SystemIdentity::default()
        },
    )
}

fn bluefield2() -> PlatformIdentity {
    with_chassis(
        identity("Nvidia", Some("Nvidia-BMCMezz")),
        ChassisIdentity {
            id: "Card1".to_string(),
            model: Some("Bluefield 2 DPU 25GbE".to_string()),
            ..ChassisIdentity::default()
        },
    )
}

/// Lenovo SR675 V3 OVX at manager firmware 9.10 and the given BIOS version.
fn sr675_v3_ovx(bios_version: &str) -> PlatformIdentity {
    let mut platform = with_system(
        identity("Lenovo", None),
        SystemIdentity {
            id: "1".to_string(),
            sku: Some("7D9RCTOLWW".to_string()),
            bios_version: Some(bios_version.to_string()),
            ..SystemIdentity::default()
        },
    );
    platform.manager = Some(ManagerIdentity {
        id: "1".to_string(),
        model: Some("XCC".to_string()),
        firmware: Some("9.10".to_string()),
    });
    platform
}

fn driver(driver: Driver) -> CapabilitySelection {
    CapabilitySelection::Driver(driver)
}

/// Resolves with the compiled catalogue's defaults, as the runtime does.
fn resolve(identity: &PlatformIdentity) -> ResolvedSelection {
    built_in_rules()
        .resolve(identity, &Drivers::default_map())
        .expect("rules resolve")
}

#[test]
fn every_supported_rule_resolves_to_a_complete_compiled_map() {
    let identities = [
        nvswitch(),
        delta(),
        liteon(),
        lenovo_ami(),
        identity("Dell", None),
        identity("HPE", None),
        identity("Lenovo", None),
        identity("Supermicro", Some("Super Server")),
        identity("AMI", Some("AMI Redfish Server")),
        identity("Supermicro", Some("GB NVL")),
        identity("NVIDIA", Some("GB BMC")),
        identity("NVIDIA", Some("VR NVL72")),
        identity("NVIDIA", Some("P3809")),
        identity("NVIDIA", Some("BlueField-4")),
        identity("WIWYNN", Some("GB200 NVL")),
        lenovo_gb300(),
        viking(),
    ];

    for identity in identities {
        Drivers::validate_map(&resolve(&identity).drivers)
            .expect("rule map names only compiled capability drivers");
    }
}

#[test]
fn narrower_identity_outranks_broader_rules() {
    struct Case {
        scenario: &'static str,
        identity: PlatformIdentity,
        expect: Vec<(Capability, CapabilitySelection)>,
    }
    let cases = [
        Case {
            scenario: "Supermicro GB NVL tray takes the OpenBMC drivers",
            identity: identity("Supermicro", Some("GB NVL")),
            expect: vec![
                (Power, driver(NvidiaOpenBmcPower)),
                (Lockdown, driver(NvidiaOpenBmcLockdown)),
                (BmcControl, Standard),
                (Console, driver(NvidiaOpenBmcConsole)),
                (Bios, driver(NvidiaGbx00Bios)),
            ],
        },
        Case {
            scenario: "Supermicro GB300",
            identity: supermicro_gb300(),
            expect: vec![
                (Bios, driver(SupermicroGb300Bios)),
                (SecureBoot, Unsupported),
            ],
        },
        Case {
            scenario: "DGX Viking",
            identity: viking(),
            expect: vec![
                (Accounts, driver(NvidiaVikingAccounts)),
                (Bios, driver(NvidiaVikingBios)),
            ],
        },
        Case {
            scenario: "DGX without a BMC manager is plain AMI",
            identity: dgx_without_bmc_manager(),
            expect: vec![(Bios, driver(AmiMegaRacBios))],
        },
        Case {
            scenario: "Supermicro ARS-121L",
            identity: ars121l(),
            expect: vec![(Lockdown, driver(SupermicroArs121lLockdown))],
        },
        Case {
            scenario: "BlueField-2",
            identity: bluefield2(),
            expect: vec![(Dpu, driver(NvidiaBlueField2Dpu))],
        },
        Case {
            scenario: "BlueField-4",
            identity: identity("Nvidia", Some("BlueField-4")),
            expect: vec![
                (Dpu, driver(NvidiaBlueField4Dpu)),
                (BootOrder, driver(NvidiaBlueFieldBootOrder)),
            ],
        },
        Case {
            scenario: "NVSwitch tray shadows GH200",
            identity: nvswitch(),
            expect: vec![
                (Accounts, driver(NvidiaSwitchAccounts)),
                (Bios, driver(NvidiaSwitchBios)),
                (Power, Standard),
                (Lockdown, driver(NoopLockdown)),
            ],
        },
        Case {
            scenario: "GH200",
            identity: identity("NVIDIA", Some("P3809")),
            expect: vec![
                (Accounts, driver(NvidiaGh200Accounts)),
                (Power, driver(NvidiaGh200Power)),
                (Firmware, Standard),
                (Bios, driver(NvidiaGh200Bios)),
            ],
        },
        Case {
            scenario: "Delta power shelf",
            identity: delta(),
            expect: vec![(Lockdown, driver(NoopLockdown))],
        },
        Case {
            scenario: "Lenovo AMI shadows XCC",
            identity: lenovo_ami(),
            expect: vec![
                (Lockdown, driver(LenovoAmiLockdown)),
                (Accounts, driver(AmiMegaRacAccounts)),
                (Power, Standard),
            ],
        },
        Case {
            scenario: "Lenovo XCC",
            identity: identity("Lenovo", None),
            expect: vec![(Lockdown, driver(LenovoXccLockdown))],
        },
        Case {
            scenario: "Lenovo GB300",
            identity: lenovo_gb300(),
            expect: vec![
                (Lockdown, driver(LenovoGb300Lockdown)),
                (Bios, driver(LenovoGb300Bios)),
                (Attestation, driver(NvidiaHgxAttestation)),
            ],
        },
        Case {
            scenario: "SR675 V3 OVX workaround at its exact firmware pair",
            identity: sr675_v3_ovx("7.10"),
            expect: vec![(Power, driver(LenovoSr675V3OvxPower))],
        },
        Case {
            scenario: "SR675 V3 OVX on another BIOS keeps XCC power",
            identity: sr675_v3_ovx("7.11"),
            expect: vec![(Power, driver(LenovoXccPower))],
        },
    ];

    for case in cases {
        let resolved = resolve(&case.identity);
        for (capability, expected) in case.expect {
            assert_eq!(
                resolved.drivers.get(capability),
                &expected,
                "{}: {capability}",
                case.scenario
            );
        }
    }
}
