/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::{Capability, CapabilitySelection};
use bmc_runtime::{
    IdentityField, IdentityMatcher, MatchPattern, RuleSet, RuleSetError, SelectionRule,
};

/// The compiled selection rules mapping platform identity to capability drivers.
///
/// Tests prove every rule names a driver in [`crate::drivers`].
pub fn built_in_rules() -> RuleSet {
    RuleSet::new(built_ins()).expect("compiled selection rules must be valid")
}

/// The compiled rules plus deployment overrides written as TOML `[[rules]]`.
///
/// Parsed rules rank as deployment overrides, so they outrank every built-in
/// rule and can never be shadowed by one.
pub fn rules_with_overrides(overrides: &str) -> Result<RuleSet, RuleSetError> {
    RuleSet::with_overrides(built_ins(), overrides)
}

fn built_ins() -> Vec<SelectionRule> {
    let mut rules = Vec::new();

    family(
        &mut rules,
        "dell",
        vec![service_root_vendor("Dell")],
        &[
            (Capability::Power, "dell-idrac-power"),
            (Capability::BmcControl, "dell-idrac-bmc-control"),
            (Capability::Bios, "dell-idrac-bios"),
            (Capability::BootOrder, "dell-idrac-boot-order"),
            (Capability::Lockdown, "dell-idrac-lockdown"),
            (Capability::Accounts, "dell-idrac-accounts"),
            (Capability::Firmware, "dell-idrac-firmware"),
            (Capability::Storage, "dell-idrac-boss-storage"),
            (Capability::Console, "dell-idrac-console"),
        ],
    );
    family(
        &mut rules,
        "hpe",
        vec![service_root_vendor("HPE")],
        &[
            (Capability::Power, "hpe-ilo-power"),
            (Capability::BmcControl, "hpe-ilo-bmc-control"),
            (Capability::BootOrder, "hpe-ilo-boot-order"),
            (Capability::Lockdown, "hpe-ilo-lockdown"),
            (Capability::Accounts, "hpe-ilo-accounts"),
            (Capability::Console, "hpe-ilo-console"),
        ],
    );
    family(
        &mut rules,
        "lenovo-xcc",
        vec![service_root_vendor("Lenovo")],
        &[
            (Capability::Power, "lenovo-xcc-power"),
            (Capability::Bios, "lenovo-xcc-bios"),
            (Capability::Lockdown, "lenovo-xcc-lockdown"),
            (Capability::Accounts, "lenovo-xcc-accounts"),
            (Capability::Console, "lenovo-xcc-console"),
        ],
    );
    family(
        &mut rules,
        "supermicro",
        vec![service_root_vendor("Supermicro")],
        &[
            (Capability::Power, "supermicro-smc-power"),
            (Capability::BmcControl, "supermicro-smc-bmc-control"),
            (Capability::BootOrder, "supermicro-x13-boot-order"),
            (Capability::Lockdown, "supermicro-smc-lockdown"),
            (Capability::Console, "supermicro-bmc-console"),
        ],
    );
    family(
        &mut rules,
        "ami-megarac",
        vec![service_root_vendor("AMI")],
        &[
            (Capability::BmcControl, "ami-megarac-bmc-control"),
            (Capability::Bios, "ami-megarac-bios"),
            (Capability::Lockdown, "ami-megarac-lockdown"),
            (Capability::Accounts, "ami-megarac-accounts"),
            (Capability::Console, "ami-megarac-console"),
        ],
    );

    // Lenovo HS350x-class trays run AMI firmware behind a Lenovo service root;
    // the extra OEM-key matcher outranks the plain XCC vendor rule.
    family(
        &mut rules,
        "lenovo-ami",
        vec![service_root_vendor("Lenovo"), service_root_oem_key("Ami")],
        &[
            (Capability::BmcControl, "ami-megarac-bmc-control"),
            (Capability::Bios, "ami-megarac-bios"),
            (Capability::Lockdown, "lenovo-ami-lockdown"),
            (Capability::Accounts, "ami-megarac-accounts"),
            (Capability::Firmware, "ami-megarac-firmware"),
            (Capability::Console, "lenovo-ami-console"),
        ],
    );

    family(
        &mut rules,
        "nvidia-gbx00",
        vec![service_root_product(&["GB BMC", "GB200 NVL", "GB NVL"])],
        &[
            (Capability::Power, "nvidia-openbmc-power"),
            (Capability::Bios, "nvidia-openbmc-bios"),
            (Capability::BootOrder, "nvidia-openbmc-boot-order"),
            (Capability::Firmware, "nvidia-openbmc-firmware"),
            (Capability::Lockdown, "nvidia-openbmc-lockdown"),
            (Capability::Accounts, "nvidia-openbmc-accounts"),
            (Capability::Attestation, "nvidia-hgx-attestation"),
        ],
    );
    family(
        &mut rules,
        "nvidia-vera",
        vec![service_root_product(&["VR NVL72"])],
        &[
            (Capability::Power, "nvidia-openbmc-power"),
            (Capability::Bios, "nvidia-openbmc-bios"),
            (Capability::BootOrder, "nvidia-openbmc-boot-order"),
            (Capability::Firmware, "nvidia-openbmc-firmware"),
            (Capability::Lockdown, "nvidia-openbmc-lockdown"),
            (Capability::Accounts, "nvidia-openbmc-accounts"),
            (Capability::Attestation, "nvidia-hgx-attestation"),
        ],
    );
    family(
        &mut rules,
        "nvidia-gh",
        vec![service_root_product(&["P3809"])],
        &[
            (Capability::Power, "nvidia-openbmc-power"),
            (Capability::Bios, "nvidia-openbmc-bios"),
            (Capability::BootOrder, "nvidia-openbmc-boot-order"),
            (Capability::Firmware, "nvidia-openbmc-firmware"),
            (Capability::Lockdown, "nvidia-openbmc-lockdown"),
            (Capability::Accounts, "nvidia-openbmc-accounts"),
        ],
    );
    rules.push(SelectionRule::new(
        "nvidia-gh-attestation",
        Capability::Attestation,
        vec![service_root_product(&["P3809"])],
        CapabilitySelection::Unsupported,
    ));
    family(
        &mut rules,
        "bluefield",
        vec![service_root_product(&[
            "Nvidia-BMCMezz",
            "BlueField-3 DPU",
            "BlueField-4",
            "B4240V",
        ])],
        &[
            (Capability::Bios, "nvidia-bluefield-bios"),
            (Capability::Accounts, "nvidia-bluefield-accounts"),
            (Capability::Dpu, "nvidia-bluefield-dpu"),
            (Capability::Console, "nvidia-bluefield-console"),
        ],
    );

    // GB NVSwitch trays share the GH200 service root; only their chassis ids
    // tell them apart. They have no BIOS, secure boot, lockdown, or attestation.
    let nvswitch = || vec![contains(IdentityField::ChassisId, "NVSwitch")];
    family(
        &mut rules,
        "nvidia-switch",
        nvswitch(),
        &[
            (Capability::BootOrder, "nvidia-openbmc-boot-order"),
            (Capability::Accounts, "nvidia-switch-accounts"),
        ],
    );
    unsupported(
        &mut rules,
        "nvidia-switch",
        nvswitch(),
        &[
            Capability::Bios,
            Capability::SecureBoot,
            Capability::Lockdown,
            Capability::Firmware,
            Capability::Attestation,
        ],
    );
    let delta = || {
        vec![contains_any_case(
            IdentityField::ChassisManufacturer,
            "Delta",
        )]
    };
    family(
        &mut rules,
        "delta-power-shelf",
        delta(),
        &[
            (Capability::Power, "delta-power-shelf-power"),
            (Capability::Accounts, "delta-power-shelf-accounts"),
        ],
    );
    unsupported(
        &mut rules,
        "delta-power-shelf",
        delta(),
        &[
            Capability::Bios,
            Capability::BootOrder,
            Capability::SecureBoot,
            Capability::Attestation,
        ],
    );
    let liteon = || {
        vec![contains_any_case(
            IdentityField::ChassisManufacturer,
            "Lite-On",
        )]
    };
    family(
        &mut rules,
        "liteon-power-shelf",
        liteon(),
        &[
            (Capability::Power, "liteon-power-shelf-power"),
            (Capability::Accounts, "liteon-power-shelf-accounts"),
        ],
    );
    unsupported(
        &mut rules,
        "liteon-power-shelf",
        liteon(),
        &[
            Capability::BootOrder,
            Capability::SecureBoot,
            Capability::Attestation,
        ],
    );

    // SR650 V4 cuts DPU power on a Redfish restart, so the host restarts over IPMI.
    family(
        &mut rules,
        "lenovo-sr650-v4",
        vec![
            service_root_vendor("Lenovo"),
            contains(IdentityField::SystemModel, "SR650 V4"),
        ],
        &[(Capability::Power, "lenovo-sr650-v4-power")],
    );
    // ARS-121L-DNR loses BMC reachability when its host interface is disabled.
    family(
        &mut rules,
        "supermicro-ars121l",
        vec![
            service_root_vendor("Supermicro"),
            contains(IdentityField::SystemModel, "ARS-121L-DNR"),
        ],
        &[(Capability::Lockdown, "supermicro-ars121l-lockdown")],
    );
    // BlueField-2 identifies itself only through its card chassis model.
    family(
        &mut rules,
        "nvidia-bluefield2",
        vec![contains_any_case(
            IdentityField::ChassisModel,
            "BlueField 2",
        )],
        &[(Capability::Dpu, "nvidia-bluefield2-dpu")],
    );
    family(
        &mut rules,
        "lenovo-gb300",
        vec![
            contains(IdentityField::SystemManufacturer, "Lenovo"),
            contains(IdentityField::SystemModel, "GB300"),
        ],
        &[
            (Capability::Bios, "ami-megarac-bios"),
            (Capability::Lockdown, "lenovo-gb300-lockdown"),
            (Capability::Console, "lenovo-gb300-console"),
        ],
    );
    family(
        &mut rules,
        "nvidia-viking",
        vec![exact_value(IdentityField::SystemId, "DGX")],
        &[
            (Capability::Power, "nvidia-viking-power"),
            (Capability::BmcControl, "ami-megarac-bmc-control"),
            (Capability::Bios, "nvidia-viking-bios"),
            (Capability::Lockdown, "nvidia-viking-lockdown"),
            (Capability::Accounts, "nvidia-viking-accounts"),
            (Capability::Firmware, "nvidia-viking-firmware"),
            (Capability::Console, "nvidia-viking-console"),
        ],
    );
    family(
        &mut rules,
        "lenovo-sr675-v3-ovx",
        vec![
            exact_value(IdentityField::SystemSku, "7D9RCTOLWW"),
            exact_value(IdentityField::ManagerFirmware, "9.10"),
            exact_value(IdentityField::SystemBiosVersion, "7.10"),
        ],
        &[(Capability::Power, "lenovo-sr675-v3-ovx-power")],
    );
    rules
}

fn family(
    rules: &mut Vec<SelectionRule>,
    family: &str,
    matchers: Vec<IdentityMatcher>,
    selections: &[(Capability, &str)],
) {
    for (capability, driver) in selections {
        rules.push(SelectionRule::new(
            format!("{family}-{capability}"),
            *capability,
            matchers.clone(),
            CapabilitySelection::Driver(driver.parse().expect("compiled driver id must be valid")),
        ));
    }
}

/// Marks capabilities the hardware cannot provide so callers fail before any I/O.
fn unsupported(
    rules: &mut Vec<SelectionRule>,
    family: &str,
    matchers: Vec<IdentityMatcher>,
    capabilities: &[Capability],
) {
    for capability in capabilities {
        rules.push(SelectionRule::new(
            format!("{family}-{capability}"),
            *capability,
            matchers.clone(),
            CapabilitySelection::Unsupported,
        ));
    }
}

fn service_root_vendor(value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootVendor,
        MatchPattern::ExactAsciiCaseInsensitive(value.to_string()),
    )
}

fn service_root_oem_key(value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootOemKey,
        MatchPattern::ExactAsciiCaseInsensitive(value.to_string()),
    )
}

fn service_root_product(values: &[&str]) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootProduct,
        MatchPattern::OneOf(values.iter().map(|value| (*value).to_string()).collect()),
    )
}

fn exact_value(field: IdentityField, value: &str) -> IdentityMatcher {
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
        CapabilitySelection, ChassisIdentity, ManagerIdentity, PlatformIdentity,
        ServiceRootIdentity, SystemIdentity,
    };
    use bmc_runtime::ResolvedSelection;

    use super::*;

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

    fn driver(id: &str) -> CapabilitySelection {
        CapabilitySelection::Driver(id.parse().expect("valid driver id"))
    }

    /// Resolves with the compiled table's defaults, as the runtime does.
    fn resolve(rules: &RuleSet, identity: &PlatformIdentity) -> ResolvedSelection {
        let table = crate::drivers::<bmc_mock::test_support::TestBmc>();
        rules
            .resolve(identity, &table.default_map())
            .expect("rules resolve")
    }

    #[test]
    fn every_supported_family_resolves_to_a_complete_compiled_map() {
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
        let drivers = crate::drivers::<bmc_mock::test_support::TestBmc>();

        for identity in identities {
            let resolved = resolve(&rules, &identity);
            drivers
                .validate_map(&resolved.drivers)
                .expect("family map names only compiled capability drivers");
        }
    }

    #[test]
    fn narrower_identity_outranks_broader_rules() {
        let rules = built_in_rules();

        let gb_platform = resolve(&rules, &identity("Supermicro", Some("GB NVL")));
        assert_eq!(
            gb_platform.drivers.get(Capability::Power),
            &driver("nvidia-openbmc-power")
        );
        assert_eq!(
            gb_platform.drivers.get(Capability::Lockdown),
            &driver("nvidia-openbmc-lockdown")
        );

        let mut viking = identity("AMI", None);
        viking.system = Some(SystemIdentity {
            id: "DGX".to_string(),
            ..SystemIdentity::default()
        });
        let viking = resolve(&rules, &viking);
        assert_eq!(
            viking.drivers.get(Capability::Accounts),
            &driver("nvidia-viking-accounts")
        );
        assert_eq!(
            viking.drivers.get(Capability::Bios),
            &driver("nvidia-viking-bios")
        );

        let mut ars = identity("Supermicro", Some("Super Server"));
        ars.system = Some(SystemIdentity {
            id: "1".to_string(),
            model: Some("ARS-121L-DNR".to_string()),
            ..SystemIdentity::default()
        });
        assert_eq!(
            resolve(&rules, &ars).drivers.get(Capability::Lockdown),
            &driver("supermicro-ars121l-lockdown")
        );
        let mut bluefield2 = identity("Nvidia", Some("Nvidia-BMCMezz"));
        bluefield2.chassis = vec![ChassisIdentity {
            id: "Card1".to_string(),
            model: Some("Bluefield 2 DPU 25GbE".to_string()),
            ..ChassisIdentity::default()
        }];
        assert_eq!(
            resolve(&rules, &bluefield2).drivers.get(Capability::Dpu),
            &driver("nvidia-bluefield2-dpu")
        );
        let mut nvswitch = identity("NVIDIA", Some("P3809"));
        nvswitch.chassis = vec![ChassisIdentity {
            id: "MGX_NVSwitch_0".to_string(),
            ..ChassisIdentity::default()
        }];
        let nvswitch = resolve(&rules, &nvswitch);
        assert_eq!(
            nvswitch.drivers.get(Capability::Accounts),
            &driver("nvidia-switch-accounts")
        );
        assert_eq!(
            nvswitch.drivers.get(Capability::Bios),
            &CapabilitySelection::Unsupported
        );
        assert_eq!(
            nvswitch.drivers.get(Capability::Power),
            &driver("nvidia-openbmc-power")
        );

        let mut lenovo_ami = identity("Lenovo", None);
        lenovo_ami.service_root.oem_keys = vec!["Ami".to_string()];
        let lenovo_ami = resolve(&rules, &lenovo_ami);
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Lockdown),
            &driver("lenovo-ami-lockdown")
        );
        assert_eq!(
            lenovo_ami.drivers.get(Capability::Accounts),
            &driver("ami-megarac-accounts")
        );
        assert_eq!(
            resolve(&rules, &identity("Lenovo", None))
                .drivers
                .get(Capability::Lockdown),
            &driver("lenovo-xcc-lockdown")
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
            &driver("lenovo-sr675-v3-ovx-power")
        );
        platform
            .system
            .as_mut()
            .expect("system exists")
            .bios_version = Some("7.11".to_string());
        assert_eq!(
            resolve(&rules, &platform).drivers.get(Capability::Power),
            &driver("lenovo-xcc-power")
        );
    }
}
