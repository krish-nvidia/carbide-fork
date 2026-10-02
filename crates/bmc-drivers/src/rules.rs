/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! The compiled platform rules mapping identity to capability drivers.
//!
//! Rules are declared from broad to narrow: vendor-wide evidence first,
//! then BMC product, then exact model or firmware. Precedence is derived from
//! the identity fields a rule reads, so declaration order carries no weight;
//! it only keeps the file readable. Tests prove every rule names a compiled
//! driver of the right capability.

use bmc_platform::{Capability, IdentityField, IdentityMatcher, MatchPattern};

use crate::drivers::Driver::{self, *};
use crate::selection::{Rule, RuleError, Rules};

/// The compiled rules.
pub fn built_in_rules() -> Rules {
    Rules::new(built_ins()).expect("compiled rules must be valid")
}

/// The compiled rules plus deployment overrides written as TOML `[[rules]]`.
///
/// Parsed rules rank as deployment overrides, so they outrank every
/// built-in rule and can never be shadowed by one.
pub fn rules_with_overrides(overrides: &str) -> Result<Rules, RuleError> {
    Rules::with_overrides(built_ins(), overrides)
}

fn built_ins() -> Vec<Rule> {
    vec![
        // ---- Vendor rules: ServiceRoot vendor or chassis manufacturer ----
        Rule::new("dell", [vendor("Dell")]).drivers([
            DellIdracPower,
            DellIdracBmcControl,
            DellIdracBios,
            DellIdracBootOrder,
            DellIdracLockdown,
            DellIdracAccounts,
            DellIdracFirmware,
            DellIdracBossStorage,
            DellIdracConsole,
        ]),
        Rule::new("hpe", [vendor("HPE")]).drivers([
            HpeIloPower,
            HpeIloBmcControl,
            HpeIloBios,
            HpeIloBootOrder,
            HpeIloLockdown,
            HpeIloAccounts,
            HpeIloConsole,
        ]),
        Rule::new("lenovo-xcc", [vendor("Lenovo")]).drivers([
            LenovoXccPower,
            LenovoXccBios,
            LenovoXccLockdown,
            LenovoXccAccounts,
            LenovoXccConsole,
        ]),
        Rule::new("supermicro", [vendor("Supermicro")]).drivers([
            SupermicroSmcPower,
            SupermicroSmcBmcControl,
            SupermicroSmcBios,
            SupermicroX13BootOrder,
            SupermicroSmcLockdown,
            SupermicroBmcConsole,
        ]),
        Rule::new("ami-megarac", [vendor("AMI")]).drivers([
            AmiMegaRacBmcControl,
            AmiMegaRacBios,
            AmiMegaRacBootOrder,
            AmiMegaRacLockdown,
            AmiMegaRacConsole,
        ]),
        // Lenovo HS350x-class trays run AMI firmware behind a Lenovo service
        // root; the extra OEM-key matcher outranks the plain XCC vendor
        // rule. The AMI firmware takes the standard lockout policy and
        // standard resets, not XCC's, and has no XCC AC power cycle.
        Rule::new("lenovo-ami", [vendor("Lenovo"), oem_key("Ami")])
            .drivers([
                AmiMegaRacBmcControl,
                AmiMegaRacBios,
                AmiMegaRacBootOrder,
                LenovoAmiLockdown,
                AmiMegaRacFirmware,
                LenovoAmiConsole,
            ])
            .standard([Capability::Accounts, Capability::Power]),
        // Power shelves have no ServiceRoot vendor; their chassis manufacturer
        // identifies them.
        Rule::new("delta-power-shelf", [chassis_manufacturer("Delta")])
            .drivers([DeltaPowerShelfPower, DeltaPowerShelfAccounts, NoopLockdown])
            .unsupported([
                Capability::Bios,
                Capability::BootOrder,
                Capability::SecureBoot,
                Capability::Firmware,
                Capability::Attestation,
            ]),
        Rule::new("liteon-power-shelf", [chassis_manufacturer("Lite-On")])
            .drivers([
                LiteOnPowerShelfPower,
                LiteOnPowerShelfAccounts,
                NoopLockdown,
            ])
            .unsupported([
                Capability::Bios,
                Capability::BootOrder,
                Capability::SecureBoot,
                Capability::Attestation,
            ]),
        // Wiwynn's GB200 NVL trays run NVIDIA OpenBMC under their own vendor.
        Rule::new("nvidia-gbx00-wiwynn", [vendor("Wiwynn")])
            .drivers(OPENBMC_TRAY)
            .drivers([
                NvidiaOpenBmcPower,
                NvidiaOpenBmcFirmware,
                NvidiaOpenBmcAccounts,
                NvidiaHgxAttestation,
            ]),
        // ---- Product rules: ServiceRoot product ----
        Rule::new(
            "nvidia-gbx00",
            [product(&["GB BMC", "GB200 NVL", "GB NVL"])],
        )
        .drivers(OPENBMC_TRAY)
        .drivers([
            NvidiaOpenBmcPower,
            NvidiaOpenBmcFirmware,
            NvidiaOpenBmcAccounts,
            NvidiaHgxAttestation,
        ])
        // Supermicro's GB NVL trays also match the Supermicro vendor rule,
        // whose OEM factory reset and console do not apply to OpenBMC.
        .standard([Capability::BmcControl])
        .unsupported([Capability::Console]),
        Rule::new("nvidia-vera", [product(&["VR NVL72"])])
            .drivers(OPENBMC_TRAY)
            .drivers([
                NvidiaOpenBmcPower,
                NvidiaOpenBmcFirmware,
                NvidiaOpenBmcAccounts,
                NvidiaHgxAttestation,
            ]),
        // GH200 firmware updates take the caller's parameters unchanged.
        Rule::new("nvidia-gh", [product(&["P3809"])])
            .drivers(OPENBMC_TRAY)
            .drivers([NvidiaGh200Power, NvidiaGh200Accounts])
            .unsupported([Capability::Attestation]),
        Rule::new(
            "bluefield",
            [product(&[
                "Nvidia-BMCMezz",
                "BlueField-3 DPU",
                "BlueField-4",
                "B4240V",
            ])],
        )
        .drivers([
            NvidiaBlueFieldBios,
            NvidiaBlueFieldBootOrder,
            NvidiaBlueFieldAccounts,
            NvidiaBlueFieldDpu,
            NvidiaBlueFieldConsole,
        ]),
        // BlueField-4 keeps its DPU mode and host privileges on the network
        // adapter rather than the system and BIOS.
        Rule::new(
            "nvidia-bluefield4",
            [vendor("Nvidia"), product(&["BlueField-4", "B4240V"])],
        )
        .drivers([NvidiaBlueField4Dpu]),
        // ---- Model rules: exact system, chassis, or firmware evidence ----
        // GB NVSwitch trays share the GH200 service root; only their switch
        // chassis tells them apart. Their BIOS takes only password changes, and
        // they have no secure boot, lockdown, AC power cycle, or attestation.
        Rule::new(
            "nvidia-switch",
            [
                product(&["P3809"]),
                exact(IdentityField::ChassisId, "MGX_NVSwitch_0"),
            ],
        )
        .drivers([
            NvidiaOpenBmcBootOrder,
            NvidiaSwitchAccounts,
            NvidiaSwitchBios,
            NoopLockdown,
        ])
        .standard([Capability::Power])
        .unsupported([
            Capability::SecureBoot,
            Capability::Firmware,
            Capability::Attestation,
        ]),
        // SR650 V4 cuts DPU power on a Redfish restart, so the host restarts over IPMI.
        Rule::new(
            "lenovo-sr650-v4",
            [
                vendor("Lenovo"),
                contains(IdentityField::SystemModel, "SR650 V4"),
            ],
        )
        .drivers([LenovoSr650V4Power]),
        // ARS-121L-DNR loses BMC reachability when its host interface is disabled.
        Rule::new(
            "supermicro-ars121l",
            [
                vendor("Supermicro"),
                contains(IdentityField::SystemModel, "ARS-121L-DNR"),
            ],
        )
        .drivers([SupermicroArs121lLockdown]),
        // BlueField-2 identifies itself only through its card chassis model.
        Rule::new(
            "nvidia-bluefield2",
            [contains_any_case(
                IdentityField::ChassisModel,
                "BlueField 2",
            )],
        )
        .drivers([NvidiaBlueField2Dpu]),
        // Lenovo GB300 trays run AMI firmware; the GB300 model is on the GPU
        // baseboard, not the selected Lenovo host system.
        Rule::new(
            "lenovo-gb300",
            [
                vendor("AMI"),
                contains(IdentityField::SystemManufacturer, "Lenovo"),
                contains(IdentityField::ChassisModel, "GB300"),
            ],
        )
        .drivers([AmiMegaRacBios, LenovoGb300Lockdown, LenovoGb300Console]),
        // DGX Viking runs AMI firmware and identifies itself by its system and
        // manager ids.
        Rule::new(
            "nvidia-viking",
            [
                vendor("AMI"),
                exact(IdentityField::SystemId, "DGX"),
                exact(IdentityField::ManagerId, "BMC"),
            ],
        )
        .drivers([
            NvidiaVikingPower,
            AmiMegaRacBmcControl,
            NvidiaVikingBios,
            NvidiaVikingBootOrder,
            NvidiaVikingLockdown,
            NvidiaVikingAccounts,
            NvidiaVikingFirmware,
            NvidiaVikingConsole,
        ]),
        // Supermicro GB300 firmware exposes SecureBoot without SecureBootEnable.
        Rule::new(
            "supermicro-gb300",
            [
                vendor("Supermicro"),
                contains_any_case(IdentityField::SystemManufacturer, "Supermicro"),
                contains(IdentityField::SystemModel, "GB300"),
            ],
        )
        .unsupported([Capability::SecureBoot]),
        // A standard ForceRestart can hang on this SKU at exactly this firmware pair.
        Rule::new(
            "lenovo-sr675-v3-ovx",
            [
                exact(IdentityField::SystemSku, "7D9RCTOLWW"),
                exact(IdentityField::ManagerFirmware, "9.10"),
                exact(IdentityField::SystemBiosVersion, "7.10"),
            ],
        )
        .drivers([LenovoSr675V3OvxPower]),
    ]
}

/// The drivers every NVIDIA OpenBMC tray shares, GH200 included.
const OPENBMC_TRAY: [Driver; 3] = [
    NvidiaOpenBmcBios,
    NvidiaOpenBmcBootOrder,
    NvidiaOpenBmcLockdown,
];

fn vendor(value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootVendor,
        MatchPattern::ExactAsciiCaseInsensitive(value.to_string()),
    )
}

fn oem_key(value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootOemKey,
        MatchPattern::ExactAsciiCaseInsensitive(value.to_string()),
    )
}

fn product(values: &[&str]) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootProduct,
        MatchPattern::OneOf(values.iter().map(|value| (*value).to_string()).collect()),
    )
}

fn chassis_manufacturer(value: &str) -> IdentityMatcher {
    contains_any_case(IdentityField::ChassisManufacturer, value)
}

fn exact(field: IdentityField, value: &str) -> IdentityMatcher {
    IdentityMatcher::new(field, MatchPattern::Exact(value.to_string()))
}

fn contains(field: IdentityField, value: &str) -> IdentityMatcher {
    IdentityMatcher::new(field, MatchPattern::Contains(value.to_string()))
}

fn contains_any_case(field: IdentityField, value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        field,
        MatchPattern::ContainsAsciiCaseInsensitive(value.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use bmc_platform::{
        ChassisIdentity, ManagerIdentity, PlatformIdentity, ServiceRootIdentity, SystemIdentity,
    };

    use super::*;
    use crate::drivers::Drivers;
    use crate::selection::{CapabilitySelection, ResolvedSelection};

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

    fn driver(driver: Driver) -> CapabilitySelection {
        CapabilitySelection::Driver(driver)
    }

    /// Resolves with the compiled catalogue's defaults, as the runtime does.
    fn resolve(rules: &Rules, identity: &PlatformIdentity) -> ResolvedSelection {
        rules
            .resolve(identity, &Drivers::default_map())
            .expect("rules resolve")
    }

    #[test]
    fn every_supported_rule_resolves_to_a_complete_compiled_map() {
        let mut lenovo_gb300 = identity("AMI", Some("AMI Redfish Server"));
        lenovo_gb300.system = Some(SystemIdentity {
            id: "System_0".to_string(),
            manufacturer: Some("Lenovo".to_string()),
            model: Some("HG634N_V2".to_string()),
            ..SystemIdentity::default()
        });
        lenovo_gb300.chassis = vec![ChassisIdentity {
            id: "HGX_Chassis_0".to_string(),
            model: Some("NVIDIA GB300".to_string()),
            ..ChassisIdentity::default()
        }];
        let mut viking = identity("AMI", Some("AMI Redfish Server"));
        viking.system = Some(SystemIdentity {
            id: "DGX".to_string(),
            ..SystemIdentity::default()
        });
        let mut lenovo_ami = identity("Lenovo", None);
        lenovo_ami.service_root.oem_keys = vec!["Ami".to_string()];
        let mut nvswitch = identity("NVIDIA", Some("P3809"));
        nvswitch.chassis = vec![ChassisIdentity {
            id: "MGX_NVSwitch_0".to_string(),
            ..ChassisIdentity::default()
        }];
        let mut delta = identity("Delta Electronics Inc.", None);
        delta.chassis = vec![ChassisIdentity {
            id: "chassis".to_string(),
            manufacturer: Some("DELTA".to_string()),
            ..ChassisIdentity::default()
        }];
        let mut liteon = identity("Lite-On Technology Corp.", None);
        liteon.chassis = vec![ChassisIdentity {
            id: "powershelf".to_string(),
            manufacturer: Some("LITE-ON TECHNOLOGY CORP.".to_string()),
            ..ChassisIdentity::default()
        }];
        let identities = [
            nvswitch,
            delta,
            liteon,
            lenovo_ami,
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
            lenovo_gb300,
            viking,
        ];
        let rules = built_in_rules();

        for identity in identities {
            let resolved = resolve(&rules, &identity);
            Drivers::validate_map(&resolved.drivers)
                .expect("rule map names only compiled capability drivers");
        }
    }

    #[test]
    fn narrower_identity_outranks_broader_rules() {
        let rules = built_in_rules();

        let gb_platform = resolve(&rules, &identity("Supermicro", Some("GB NVL")));
        assert_eq!(
            gb_platform.drivers.get(Capability::Power),
            &driver(NvidiaOpenBmcPower)
        );
        assert_eq!(
            gb_platform.drivers.get(Capability::Lockdown),
            &driver(NvidiaOpenBmcLockdown)
        );
        assert_eq!(
            gb_platform.drivers.get(Capability::BmcControl),
            &CapabilitySelection::Standard
        );
        assert_eq!(
            gb_platform.drivers.get(Capability::Console),
            &CapabilitySelection::Unsupported
        );

        let mut viking = identity("AMI", None);
        viking.system = Some(SystemIdentity {
            id: "DGX".to_string(),
            ..SystemIdentity::default()
        });
        let viking = resolve(&rules, &viking);
        assert_eq!(
            viking.drivers.get(Capability::Accounts),
            &driver(NvidiaVikingAccounts)
        );
        assert_eq!(
            viking.drivers.get(Capability::Bios),
            &driver(NvidiaVikingBios)
        );
        let mut dgx_without_bmc_manager = identity("AMI", None);
        dgx_without_bmc_manager.system = Some(SystemIdentity {
            id: "DGX".to_string(),
            ..SystemIdentity::default()
        });
        dgx_without_bmc_manager.manager = Some(ManagerIdentity {
            id: "Self".to_string(),
            model: None,
            firmware: None,
        });
        assert_eq!(
            resolve(&rules, &dgx_without_bmc_manager)
                .drivers
                .get(Capability::Bios),
            &driver(AmiMegaRacBios)
        );

        let mut ars = identity("Supermicro", Some("Super Server"));
        ars.system = Some(SystemIdentity {
            id: "1".to_string(),
            model: Some("ARS-121L-DNR".to_string()),
            ..SystemIdentity::default()
        });
        assert_eq!(
            resolve(&rules, &ars).drivers.get(Capability::Lockdown),
            &driver(SupermicroArs121lLockdown)
        );
        let mut bluefield2 = identity("Nvidia", Some("Nvidia-BMCMezz"));
        bluefield2.chassis = vec![ChassisIdentity {
            id: "Card1".to_string(),
            model: Some("Bluefield 2 DPU 25GbE".to_string()),
            ..ChassisIdentity::default()
        }];
        assert_eq!(
            resolve(&rules, &bluefield2).drivers.get(Capability::Dpu),
            &driver(NvidiaBlueField2Dpu)
        );
        let bluefield4 = resolve(&rules, &identity("Nvidia", Some("BlueField-4")));
        assert_eq!(
            bluefield4.drivers.get(Capability::Dpu),
            &driver(NvidiaBlueField4Dpu)
        );
        assert_eq!(
            bluefield4.drivers.get(Capability::BootOrder),
            &driver(NvidiaBlueFieldBootOrder)
        );
        let mut nvswitch = identity("NVIDIA", Some("P3809"));
        nvswitch.chassis = vec![ChassisIdentity {
            id: "MGX_NVSwitch_0".to_string(),
            ..ChassisIdentity::default()
        }];
        let nvswitch = resolve(&rules, &nvswitch);
        assert_eq!(
            nvswitch.drivers.get(Capability::Accounts),
            &driver(NvidiaSwitchAccounts)
        );
        let gh200 = resolve(&rules, &identity("NVIDIA", Some("P3809")));
        assert_eq!(
            gh200.drivers.get(Capability::Accounts),
            &driver(NvidiaGh200Accounts)
        );
        assert_eq!(
            gh200.drivers.get(Capability::Power),
            &driver(NvidiaGh200Power)
        );
        assert_eq!(
            gh200.drivers.get(Capability::Firmware),
            &CapabilitySelection::Standard
        );
        assert_eq!(
            nvswitch.drivers.get(Capability::Bios),
            &driver(NvidiaSwitchBios)
        );
        assert_eq!(
            nvswitch.drivers.get(Capability::Power),
            &CapabilitySelection::Standard
        );
        assert_eq!(
            nvswitch.drivers.get(Capability::Lockdown),
            &driver(NoopLockdown)
        );
        let mut delta = identity("Delta Electronics Inc.", None);
        delta.chassis = vec![ChassisIdentity {
            id: "chassis".to_string(),
            manufacturer: Some("DELTA".to_string()),
            ..ChassisIdentity::default()
        }];
        assert_eq!(
            resolve(&rules, &delta).drivers.get(Capability::Lockdown),
            &driver(NoopLockdown)
        );

        let mut lenovo_ami = identity("Lenovo", None);
        lenovo_ami.service_root.oem_keys = vec!["Ami".to_string()];
        let lenovo_ami = resolve(&rules, &lenovo_ami);
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Lockdown),
            &driver(LenovoAmiLockdown)
        );
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Accounts),
            &CapabilitySelection::Standard
        );
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Power),
            &CapabilitySelection::Standard
        );
        assert_eq!(
            resolve(&rules, &identity("Lenovo", None))
                .drivers
                .get(Capability::Lockdown),
            &driver(LenovoXccLockdown)
        );

        let mut lenovo_gb300 = identity("AMI", Some("AMI Redfish Server"));
        lenovo_gb300.system = Some(SystemIdentity {
            id: "System_0".to_string(),
            manufacturer: Some("Lenovo".to_string()),
            model: Some("HG634N_V2".to_string()),
            ..SystemIdentity::default()
        });
        lenovo_gb300.chassis = vec![ChassisIdentity {
            id: "HGX_Chassis_0".to_string(),
            model: Some("NVIDIA GB300".to_string()),
            ..ChassisIdentity::default()
        }];
        assert_eq!(
            resolve(&rules, &lenovo_gb300)
                .drivers
                .get(Capability::Lockdown),
            &driver(LenovoGb300Lockdown)
        );
    }

    #[test]
    fn sr675_workaround_requires_the_exact_firmware_boundary() {
        let rules = built_in_rules();
        let mut platform = identity("Lenovo", None);
        platform.manager = Some(ManagerIdentity {
            id: "1".to_string(),
            model: Some("XCC".to_string()),
            firmware: Some("9.10".to_string()),
        });
        platform.system = Some(SystemIdentity {
            id: "1".to_string(),
            sku: Some("7D9RCTOLWW".to_string()),
            bios_version: Some("7.10".to_string()),
            ..SystemIdentity::default()
        });
        assert_eq!(
            resolve(&rules, &platform).drivers.get(Capability::Power),
            &driver(LenovoSr675V3OvxPower)
        );
        platform
            .system
            .as_mut()
            .expect("system exists")
            .bios_version = Some("7.11".to_string());
        assert_eq!(
            resolve(&rules, &platform).drivers.get(Capability::Power),
            &driver(LenovoXccPower)
        );
    }
}
