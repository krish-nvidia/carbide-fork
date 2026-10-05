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

use bmc_platform::{Capability, IdentityField, IdentityMatcher as Match};

use crate::drivers::Driver::*;
use crate::selection::{Rule, RuleError, Rules};

#[cfg(test)]
mod tests;

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

#[allow(clippy::vec_init_then_push)] // rustfmt strips blank lines between `vec!` elements.
fn built_ins() -> Vec<Rule> {
    let mut rules = Vec::new();

    // ---- Vendor rules: ServiceRoot vendor or chassis manufacturer ----

    rules.push(Rule::new("dell", [Match::vendor("Dell")]).drivers([
        DellIdracPower,
        DellIdracBmcControl,
        DellIdracBios,
        DellIdracBootOrder,
        DellIdracLockdown,
        DellIdracAccounts,
        DellIdracFirmware,
        DellIdracBossStorage,
        DellIdracConsole,
    ]));

    rules.push(Rule::new("hpe", [Match::vendor("HPE")]).drivers([
        HpeIloPower,
        HpeIloBmcControl,
        HpeIloBios,
        HpeIloBootOrder,
        HpeIloLockdown,
        HpeIloAccounts,
        HpeIloConsole,
    ]));

    rules.push(Rule::new("lenovo-xcc", [Match::vendor("Lenovo")]).drivers([
        LenovoXccPower,
        LenovoXccBios,
        LenovoXccBootOrder,
        LenovoXccFirmware,
        LenovoXccLockdown,
        LenovoXccAccounts,
        LenovoXccConsole,
    ]));

    rules.push(
        Rule::new("supermicro", [Match::vendor("Supermicro")]).drivers([
            SupermicroSmcPower,
            SupermicroSmcBmcControl,
            SupermicroSmcBios,
            SupermicroSmcFirmware,
            SupermicroSmcBootOrder,
            SupermicroSmcLockdown,
            SupermicroBmcConsole,
        ]),
    );

    rules.push(Rule::new("ami-megarac", [Match::vendor("AMI")]).drivers([
        AmiMegaRacBmcControl,
        AmiMegaRacBios,
        AmiMegaRacBootOrder,
        AmiMegaRacLockdown,
        AmiMegaRacAccounts,
        AmiMegaRacConsole,
    ]));

    // Lenovo HS350x-class trays run AMI firmware behind a Lenovo service
    // root; the extra OEM-key matcher outranks the plain XCC vendor
    // rule. The AMI firmware takes AMI accounts and standard resets, not
    // XCC's, and has no XCC AC power cycle.
    rules.push(
        Rule::new(
            "lenovo-ami",
            [Match::vendor("Lenovo"), Match::oem_key("Ami")],
        )
        .drivers([
            AmiMegaRacBmcControl,
            AmiMegaRacBios,
            AmiMegaRacBootOrder,
            LenovoAmiLockdown,
            AmiMegaRacAccounts,
            AmiMegaRacFirmware,
            LenovoAmiConsole,
        ])
        .standard([Capability::Power]),
    );

    // Power shelves have no ServiceRoot vendor; their chassis manufacturer
    // identifies them.
    rules.push(
        Rule::new("delta-power-shelf", [Match::chassis_manufacturer("Delta")])
            .drivers([DeltaPowerShelfPower, DeltaPowerShelfAccounts, NoopLockdown])
            .unsupported([
                Capability::Bios,
                Capability::BootOrder,
                Capability::SecureBoot,
                Capability::Firmware,
                Capability::Attestation,
            ]),
    );

    rules.push(
        Rule::new(
            "liteon-power-shelf",
            [Match::chassis_manufacturer("Lite-On")],
        )
        .drivers([
            LiteOnPowerShelfPower,
            LiteOnPowerShelfAccounts,
            LiteOnPowerShelfBios,
            NoopLockdown,
        ])
        .unsupported([
            Capability::BootOrder,
            Capability::SecureBoot,
            Capability::Attestation,
        ]),
    );

    // Wiwynn's GB200 NVL trays run NVIDIA OpenBMC under their own vendor.
    rules.push(
        Rule::new("nvidia-gbx00-wiwynn", [Match::vendor("Wiwynn")]).drivers([
            NvidiaGbx00BootOrder,
            NvidiaOpenBmcLockdown,
            NvidiaOpenBmcPower,
            NvidiaOpenBmcFirmware,
            NvidiaOpenBmcAccounts,
            NvidiaHgxAttestation,
            NvidiaGbx00Bios,
        ]),
    );

    // ---- Product rules: ServiceRoot product ----

    // Supermicro's GB NVL trays also match the Supermicro vendor rule,
    // whose OEM factory reset and console do not apply to OpenBMC.
    rules.push(
        Rule::new(
            "nvidia-gbx00",
            [Match::product(&["GB BMC", "GB200 NVL", "GB NVL"])],
        )
        .drivers([
            NvidiaGbx00BootOrder,
            NvidiaOpenBmcLockdown,
            NvidiaOpenBmcPower,
            NvidiaOpenBmcFirmware,
            NvidiaOpenBmcAccounts,
            NvidiaHgxAttestation,
            NvidiaGbx00Bios,
        ])
        .standard([Capability::BmcControl])
        .unsupported([Capability::Console]),
    );

    rules.push(
        Rule::new("nvidia-vera", [Match::product(&["VR NVL72"])]).drivers([
            NvidiaVeraRubinBootOrder,
            NvidiaOpenBmcLockdown,
            NvidiaOpenBmcPower,
            NvidiaOpenBmcFirmware,
            NvidiaOpenBmcAccounts,
            NvidiaHgxAttestation,
            NvidiaVeraRubinBios,
        ]),
    );

    // GH200 firmware updates take the caller's parameters unchanged.
    rules.push(
        Rule::new("nvidia-gh", [Match::product(&["P3809"])])
            .drivers([
                NvidiaGh200BootOrder,
                NvidiaOpenBmcLockdown,
                NvidiaGh200Bios,
                NvidiaGh200Power,
                NvidiaGh200Accounts,
            ])
            .unsupported([Capability::Attestation]),
    );

    rules.push(
        Rule::new(
            "bluefield",
            [Match::product(&[
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
    );

    // BlueField-4 keeps its DPU mode and host privileges on the network
    // adapter rather than the system and BIOS.
    rules.push(
        Rule::new(
            "nvidia-bluefield4",
            [
                Match::vendor("Nvidia"),
                Match::product(&["BlueField-4", "B4240V"]),
            ],
        )
        .drivers([NvidiaBlueField4Dpu]),
    );

    // ---- Model rules: exact system, chassis, or firmware evidence ----

    // GB NVSwitch trays share the GH200 service root; only their switch
    // chassis tells them apart. Their BIOS takes only password changes, and
    // they have no secure boot, lockdown, AC power cycle, or attestation.
    rules.push(
        Rule::new(
            "nvidia-switch",
            [
                Match::product(&["P3809"]),
                Match::exact(IdentityField::ChassisId, "MGX_NVSwitch_0"),
            ],
        )
        .drivers([
            NvidiaSwitchBootOrder,
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
    );

    // XCC 3 has no OEM boot settings; it orders network adapters through BIOS
    // attributes.
    rules.push(
        Rule::new(
            "lenovo-xcc3",
            [
                Match::vendor("Lenovo"),
                Match::exact(IdentityField::ManagerModel, "Lenovo XClarity Controller 3"),
            ],
        )
        .drivers([LenovoXcc3BootOrder]),
    );

    // SR650 V4 cuts DPU power on a Redfish restart, so the host restarts over IPMI.
    rules.push(
        Rule::new(
            "lenovo-sr650-v4",
            [
                Match::vendor("Lenovo"),
                Match::contains(IdentityField::SystemModel, "SR650 V4"),
            ],
        )
        .drivers([LenovoSr650V4Power]),
    );

    // ARS-121L-DNR loses BMC reachability when its host interface is disabled.
    rules.push(
        Rule::new(
            "supermicro-ars121l",
            [
                Match::vendor("Supermicro"),
                Match::contains(IdentityField::SystemModel, "ARS-121L-DNR"),
            ],
        )
        .drivers([SupermicroArs121lLockdown]),
    );

    // BlueField-2 identifies itself only through its card chassis model.
    rules.push(
        Rule::new(
            "nvidia-bluefield2",
            [Match::contains_any_case(
                IdentityField::ChassisModel,
                "BlueField 2",
            )],
        )
        .drivers([NvidiaBlueField2Dpu]),
    );

    // Lenovo GB300 trays run AMI firmware; the GB300 model is on the GPU
    // baseboard, not the selected Lenovo host system. The baseboard's GPUs
    // attest through HGX ComponentIntegrity.
    rules.push(
        Rule::new(
            "lenovo-gb300",
            [
                Match::vendor("AMI"),
                Match::contains(IdentityField::SystemManufacturer, "Lenovo"),
                Match::contains(IdentityField::ChassisModel, "GB300"),
            ],
        )
        .drivers([
            LenovoGb300Bios,
            LenovoGb300BootOrder,
            LenovoGb300Lockdown,
            LenovoGb300Console,
            NvidiaHgxAttestation,
        ]),
    );

    // DGX Viking runs AMI firmware and identifies itself by its system and
    // manager ids.
    rules.push(
        Rule::new(
            "nvidia-viking",
            [
                Match::vendor("AMI"),
                Match::exact(IdentityField::SystemId, "DGX"),
                Match::exact(IdentityField::ManagerId, "BMC"),
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
    );

    // Supermicro GB300 firmware exposes SecureBoot without SecureBootEnable.
    rules.push(
        Rule::new(
            "supermicro-gb300",
            [
                Match::vendor("Supermicro"),
                Match::contains_any_case(IdentityField::SystemManufacturer, "Supermicro"),
                Match::contains(IdentityField::SystemModel, "GB300"),
            ],
        )
        .drivers([SupermicroGb300Bios])
        .unsupported([Capability::SecureBoot]),
    );

    // A standard ForceRestart can hang on this SKU at exactly this firmware pair.
    rules.push(
        Rule::new(
            "lenovo-sr675-v3-ovx",
            [
                Match::exact(IdentityField::SystemSku, "7D9RCTOLWW"),
                Match::exact(IdentityField::ManagerFirmware, "9.10"),
                Match::exact(IdentityField::SystemBiosVersion, "7.10"),
            ],
        )
        .drivers([LenovoSr675V3OvxPower]),
    );

    rules
}
