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

use bmc_platform::{Capability, EtagMode, IdentityField, IdentityMatcher, MatchPattern};

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
            SupermicroX13BootOrder,
            SupermicroSmcLockdown,
            SupermicroBmcConsole,
        ]),
        // AMI MegaRAC requires `If-Match` but rejects the ETag it served.
        Rule::new("ami-megarac", [vendor("AMI")])
            .drivers([
                AmiMegaRacBmcControl,
                AmiMegaRacBios,
                AmiMegaRacLockdown,
                AmiMegaRacConsole,
            ])
            .etag(EtagMode::Wildcard),
        // Lenovo HS350x-class trays run AMI firmware behind a Lenovo service
        // root; the extra OEM-key matcher outranks the plain XCC vendor
        // rule. The AMI firmware takes the standard lockout policy, not XCC's.
        Rule::new("lenovo-ami", [vendor("Lenovo"), oem_key("Ami")])
            .drivers([
                AmiMegaRacBmcControl,
                AmiMegaRacBios,
                LenovoAmiLockdown,
                AmiMegaRacFirmware,
                LenovoAmiConsole,
            ])
            .standard([Capability::Accounts])
            .etag(EtagMode::Wildcard),
        // Power shelves have no ServiceRoot vendor; their chassis manufacturer
        // identifies them.
        Rule::new("delta-power-shelf", [chassis_manufacturer("Delta")])
            .drivers([DeltaPowerShelfPower, DeltaPowerShelfAccounts])
            .unsupported([
                Capability::Bios,
                Capability::BootOrder,
                Capability::SecureBoot,
                Capability::Attestation,
            ]),
        Rule::new("liteon-power-shelf", [chassis_manufacturer("Lite-On")])
            .drivers([LiteOnPowerShelfPower, LiteOnPowerShelfAccounts])
            .unsupported([
                Capability::BootOrder,
                Capability::SecureBoot,
                Capability::Attestation,
            ]),
        // ---- Product rules: ServiceRoot product ----
        Rule::new(
            "nvidia-gbx00",
            [product(&["GB BMC", "GB200 NVL", "GB NVL"])],
        )
        .drivers(OPENBMC_TRAY)
        .drivers([NvidiaHgxAttestation]),
        Rule::new("nvidia-vera", [product(&["VR NVL72"])])
            .drivers(OPENBMC_TRAY)
            .drivers([NvidiaHgxAttestation]),
        Rule::new("nvidia-gh", [product(&["P3809"])])
            .drivers(OPENBMC_TRAY)
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
            NvidiaBlueFieldAccounts,
            NvidiaBlueFieldDpu,
            NvidiaBlueFieldConsole,
        ]),
        // ---- Model rules: exact system, chassis, or firmware evidence ----
        // GB NVSwitch trays share the GH200 service root; only their chassis
        // ids tell them apart. They have no BIOS, secure boot, lockdown, or
        // attestation.
        Rule::new(
            "nvidia-switch",
            [contains(IdentityField::ChassisId, "NVSwitch")],
        )
        .drivers([NvidiaOpenBmcBootOrder, NvidiaSwitchAccounts])
        .unsupported([
            Capability::Bios,
            Capability::SecureBoot,
            Capability::Lockdown,
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
        Rule::new(
            "lenovo-gb300",
            [
                contains(IdentityField::SystemManufacturer, "Lenovo"),
                contains(IdentityField::SystemModel, "GB300"),
            ],
        )
        .drivers([AmiMegaRacBios, LenovoGb300Lockdown, LenovoGb300Console]),
        // DGX Viking runs AMI firmware and identifies itself by its system id.
        Rule::new("nvidia-viking", [exact(IdentityField::SystemId, "DGX")])
            .drivers([
                NvidiaVikingPower,
                AmiMegaRacBmcControl,
                NvidiaVikingBios,
                NvidiaVikingLockdown,
                NvidiaVikingAccounts,
                NvidiaVikingFirmware,
                NvidiaVikingConsole,
            ])
            .etag(EtagMode::Wildcard),
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

/// The drivers every NVIDIA OpenBMC compute tray shares.
const OPENBMC_TRAY: [Driver; 6] = [
    NvidiaOpenBmcPower,
    NvidiaOpenBmcBios,
    NvidiaOpenBmcBootOrder,
    NvidiaOpenBmcFirmware,
    NvidiaOpenBmcLockdown,
    NvidiaOpenBmcAccounts,
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
            model: Some("ThinkSystem GB300".to_string()),
            ..SystemIdentity::default()
        });
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
        assert_eq!(gb_platform.etag_mode, EtagMode::Resource);
        assert_eq!(
            gb_platform.drivers.get(Capability::Power),
            &driver(NvidiaOpenBmcPower)
        );
        assert_eq!(
            gb_platform.drivers.get(Capability::Lockdown),
            &driver(NvidiaOpenBmcLockdown)
        );

        let mut viking = identity("AMI", None);
        viking.system = Some(SystemIdentity {
            id: "DGX".to_string(),
            ..SystemIdentity::default()
        });
        let viking = resolve(&rules, &viking);
        assert_eq!(viking.etag_mode, EtagMode::Wildcard);
        assert_eq!(
            viking.drivers.get(Capability::Accounts),
            &driver(NvidiaVikingAccounts)
        );
        assert_eq!(
            viking.drivers.get(Capability::Bios),
            &driver(NvidiaVikingBios)
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
        assert_eq!(
            nvswitch.drivers.get(Capability::Bios),
            &CapabilitySelection::Unsupported
        );
        assert_eq!(
            nvswitch.drivers.get(Capability::Power),
            &driver(NvidiaOpenBmcPower)
        );

        let mut lenovo_ami = identity("Lenovo", None);
        lenovo_ami.service_root.oem_keys = vec!["Ami".to_string()];
        let lenovo_ami = resolve(&rules, &lenovo_ami);
        assert_eq!(lenovo_ami.etag_mode, EtagMode::Wildcard);
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Lockdown),
            &driver(LenovoAmiLockdown)
        );
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Accounts),
            &CapabilitySelection::Standard
        );
        assert_eq!(
            resolve(&rules, &identity("Lenovo", None))
                .drivers
                .get(Capability::Lockdown),
            &driver(LenovoXccLockdown)
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
