/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::DriverId;
use bmc_runtime::{AnyDriver, DriverTable};
use nv_redfish::core::Bmc;

use crate::{
    accounts, attestation, bios, bmc_control, boot_order, console, dpu, firmware, lockdown, power,
    secure_boot, storage,
};

/// Builds the table of every capability driver compiled into NICo.
///
/// Standard drivers serve `CapabilitySelection::Standard`; named drivers are
/// the ids selection rules and persisted driver maps refer to. The table is
/// validated by tests, so an inconsistent entry list is a build-time defect.
pub fn drivers<B: Bmc + 'static>() -> DriverTable<B> {
    let standard = [
        AnyDriver::Power(&power::StandardPower),
        AnyDriver::BmcControl(&bmc_control::StandardBmcControl),
        AnyDriver::Bios(&bios::StandardBios),
        AnyDriver::BootOrder(&boot_order::StandardBootOrder),
        AnyDriver::SecureBoot(&secure_boot::StandardSecureBoot),
        AnyDriver::Accounts(&accounts::StandardAccounts),
        AnyDriver::Firmware(&firmware::StandardFirmware),
        AnyDriver::Attestation(&attestation::StandardAttestation),
    ]
    .into_iter()
    .map(|driver| (None, driver));
    let named = [
        ("dell-idrac-power", AnyDriver::Power(&power::IdracPower)),
        (
            "delta-power-shelf-power",
            AnyDriver::Power(&power::DeltaPowerShelfPower),
        ),
        (
            "liteon-power-shelf-power",
            AnyDriver::Power(&power::LiteOnPowerShelfPower),
        ),
        ("hpe-ilo-power", AnyDriver::Power(&power::IloPower)),
        ("lenovo-xcc-power", AnyDriver::Power(&power::XccPower)),
        (
            "lenovo-sr650-v4-power",
            AnyDriver::Power(&power::Sr650V4Power),
        ),
        (
            "lenovo-sr675-v3-ovx-power",
            AnyDriver::Power(&power::Sr675V3OvxPower),
        ),
        (
            "nvidia-openbmc-power",
            AnyDriver::Power(&power::OpenBmcPower),
        ),
        ("nvidia-viking-power", AnyDriver::Power(&power::VikingPower)),
        ("supermicro-smc-power", AnyDriver::Power(&power::SmcPower)),
        (
            "ami-megarac-bmc-control",
            AnyDriver::BmcControl(&bmc_control::MegaRacBmcControl),
        ),
        (
            "dell-idrac-bmc-control",
            AnyDriver::BmcControl(&bmc_control::IdracBmcControl),
        ),
        (
            "hpe-ilo-bmc-control",
            AnyDriver::BmcControl(&bmc_control::IloBmcControl),
        ),
        (
            "supermicro-smc-bmc-control",
            AnyDriver::BmcControl(&bmc_control::SmcBmcControl),
        ),
        ("dell-idrac-bios", AnyDriver::Bios(&bios::IdracBios)),
        ("lenovo-xcc-bios", AnyDriver::Bios(&bios::XccBios)),
        ("ami-megarac-bios", AnyDriver::Bios(&bios::MegaRacBios)),
        ("nvidia-openbmc-bios", AnyDriver::Bios(&bios::OpenBmcBios)),
        ("nvidia-viking-bios", AnyDriver::Bios(&bios::VikingBios)),
        (
            "nvidia-bluefield-bios",
            AnyDriver::Bios(&bios::BlueFieldBios),
        ),
        (
            "dell-idrac-boot-order",
            AnyDriver::BootOrder(&boot_order::IdracBootOrder),
        ),
        (
            "hpe-ilo-boot-order",
            AnyDriver::BootOrder(&boot_order::IloBootOrder),
        ),
        (
            "nvidia-openbmc-boot-order",
            AnyDriver::BootOrder(&boot_order::OpenBmcBootOrder),
        ),
        (
            "supermicro-x13-boot-order",
            AnyDriver::BootOrder(&boot_order::X13BootOrder),
        ),
        (
            "dell-idrac-lockdown",
            AnyDriver::Lockdown(&lockdown::IdracLockdown),
        ),
        (
            "hpe-ilo-lockdown",
            AnyDriver::Lockdown(&lockdown::IloLockdown),
        ),
        (
            "lenovo-xcc-lockdown",
            AnyDriver::Lockdown(&lockdown::XccLockdown),
        ),
        (
            "supermicro-smc-lockdown",
            AnyDriver::Lockdown(&lockdown::SmcLockdown),
        ),
        (
            "supermicro-ars121l-lockdown",
            AnyDriver::Lockdown(&lockdown::Ars121lLockdown),
        ),
        (
            "ami-megarac-lockdown",
            AnyDriver::Lockdown(&lockdown::MegaRacLockdown),
        ),
        (
            "lenovo-ami-lockdown",
            AnyDriver::Lockdown(&lockdown::LenovoAmiLockdown),
        ),
        (
            "lenovo-gb300-lockdown",
            AnyDriver::Lockdown(&lockdown::Gb300Lockdown),
        ),
        (
            "nvidia-viking-lockdown",
            AnyDriver::Lockdown(&lockdown::VikingLockdown),
        ),
        (
            "nvidia-openbmc-lockdown",
            AnyDriver::Lockdown(&lockdown::OpenBmcLockdown),
        ),
        (
            "dell-idrac-accounts",
            AnyDriver::Accounts(&accounts::IdracAccounts),
        ),
        (
            "hpe-ilo-accounts",
            AnyDriver::Accounts(&accounts::IloAccounts),
        ),
        (
            "lenovo-xcc-accounts",
            AnyDriver::Accounts(&accounts::XccAccounts),
        ),
        (
            "nvidia-viking-accounts",
            AnyDriver::Accounts(&accounts::VikingAccounts),
        ),
        (
            "nvidia-openbmc-accounts",
            AnyDriver::Accounts(&accounts::OpenBmcAccounts),
        ),
        (
            "nvidia-bluefield-accounts",
            AnyDriver::Accounts(&accounts::BlueFieldAccounts),
        ),
        (
            "nvidia-switch-accounts",
            AnyDriver::Accounts(&accounts::SwitchAccounts),
        ),
        (
            "delta-power-shelf-accounts",
            AnyDriver::Accounts(&accounts::DeltaPowerShelfAccounts),
        ),
        (
            "liteon-power-shelf-accounts",
            AnyDriver::Accounts(&accounts::LiteOnPowerShelfAccounts),
        ),
        (
            "dell-idrac-firmware",
            AnyDriver::Firmware(&firmware::IdracFirmware),
        ),
        (
            "ami-megarac-firmware",
            AnyDriver::Firmware(&firmware::MegaRacFirmware),
        ),
        (
            "nvidia-viking-firmware",
            AnyDriver::Firmware(&firmware::VikingFirmware),
        ),
        (
            "nvidia-openbmc-firmware",
            AnyDriver::Firmware(&firmware::OpenBmcFirmware),
        ),
        (
            "dell-idrac-boss-storage",
            AnyDriver::Storage(&storage::IdracBossStorage),
        ),
        ("nvidia-bluefield-dpu", AnyDriver::Dpu(&dpu::BlueField3Dpu)),
        ("nvidia-bluefield2-dpu", AnyDriver::Dpu(&dpu::BlueField2Dpu)),
        (
            "nvidia-hgx-attestation",
            AnyDriver::Attestation(&attestation::HgxAttestation),
        ),
        (
            "dell-idrac-console",
            AnyDriver::Console(&console::IdracConsole),
        ),
        ("hpe-ilo-console", AnyDriver::Console(&console::IloConsole)),
        (
            "lenovo-xcc-console",
            AnyDriver::Console(&console::XccConsole),
        ),
        (
            "supermicro-bmc-console",
            AnyDriver::Console(&console::SupermicroBmcConsole),
        ),
        (
            "ami-megarac-console",
            AnyDriver::Console(&console::MegaRacConsole),
        ),
        (
            "lenovo-ami-console",
            AnyDriver::Console(&console::LenovoAmiConsole),
        ),
        (
            "lenovo-gb300-console",
            AnyDriver::Console(&console::Gb300Console),
        ),
        (
            "nvidia-viking-console",
            AnyDriver::Console(&console::VikingConsole),
        ),
        (
            "nvidia-bluefield-console",
            AnyDriver::Console(&console::BlueFieldConsole),
        ),
    ]
    .into_iter()
    .map(|(id, driver)| {
        let id: DriverId = id.parse().expect("compiled driver id must be valid");
        (Some(id), driver)
    });
    DriverTable::new(standard.chain(named)).expect("compiled driver table must be consistent")
}

#[cfg(test)]
mod tests {
    use bmc_mock::test_support::TestBmc;
    use bmc_platform::{Capability, CapabilitySelection};
    use bmc_runtime::DriverTableError;

    use super::*;
    use crate::rules::built_in_rules;

    #[test]
    fn compiled_table_serves_every_rule_and_reports_missing_standards() {
        let table = drivers::<TestBmc>();
        table
            .validate_rules(&built_in_rules())
            .expect("every built-in rule names a compiled driver");
        assert_eq!(
            table
                .check(Capability::Storage, &CapabilitySelection::Standard)
                .err(),
            Some(DriverTableError::StandardNotImplemented {
                capability: Capability::Storage,
            })
        );
        assert_eq!(
            table.power(&CapabilitySelection::Unsupported).err(),
            Some(DriverTableError::Unsupported {
                capability: Capability::Power,
            })
        );
    }
}
