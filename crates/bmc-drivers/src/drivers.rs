/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::DriverId;
use bmc_runtime::{AnyDriver, DriverTable};
use nv_redfish::core::{ActionError, Bmc};

use crate::{
    accounts, attestation, bios, bmc_control, boot_order, console, dpu, firmware, lockdown, power,
    secure_boot, storage,
};

/// Builds the table of every capability driver compiled into NICo.
///
/// Standard drivers serve `CapabilitySelection::Standard`; named drivers are
/// the ids selection rules and persisted driver maps refer to. The table is
/// validated by tests, so an inconsistent entry list is a build-time defect.
pub fn drivers<B>() -> DriverTable<B>
where
    B: Bmc + 'static,
    B::Error: ActionError,
{
    let standard = [
        AnyDriver::Power(&power::STANDARD_POWER),
        AnyDriver::BmcControl(&bmc_control::STANDARD_BMC_CONTROL),
        AnyDriver::Bios(&bios::STANDARD_BIOS),
        AnyDriver::BootOrder(&boot_order::StandardBootOrder),
        AnyDriver::SecureBoot(&secure_boot::StandardSecureBoot),
        AnyDriver::Accounts(&accounts::STANDARD_ACCOUNTS),
        AnyDriver::Firmware(&firmware::STANDARD_FIRMWARE),
        AnyDriver::Attestation(&attestation::STANDARD_ATTESTATION),
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
        ("lenovo-xcc-power", AnyDriver::Power(&power::XCC_POWER)),
        (
            "lenovo-sr650-v4-power",
            AnyDriver::Power(&power::SR650_V4_POWER),
        ),
        (
            "lenovo-sr675-v3-ovx-power",
            AnyDriver::Power(&power::Sr675V3OvxPower),
        ),
        (
            "nvidia-openbmc-power",
            AnyDriver::Power(&power::OPENBMC_POWER),
        ),
        (
            "nvidia-viking-power",
            AnyDriver::Power(&power::VIKING_POWER),
        ),
        ("supermicro-smc-power", AnyDriver::Power(&power::SMC_POWER)),
        (
            "ami-megarac-bmc-control",
            AnyDriver::BmcControl(&bmc_control::MEGARAC_BMC_CONTROL),
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
            AnyDriver::BmcControl(&bmc_control::SMC_BMC_CONTROL),
        ),
        ("dell-idrac-bios", AnyDriver::Bios(&bios::IdracBios)),
        ("lenovo-xcc-bios", AnyDriver::Bios(&bios::XCC_BIOS)),
        ("ami-megarac-bios", AnyDriver::Bios(&bios::MEGARAC_BIOS)),
        ("nvidia-openbmc-bios", AnyDriver::Bios(&bios::OPENBMC_BIOS)),
        ("nvidia-viking-bios", AnyDriver::Bios(&bios::VIKING_BIOS)),
        (
            "nvidia-bluefield-bios",
            AnyDriver::Bios(&bios::BLUEFIELD_BIOS),
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
            AnyDriver::Lockdown(&lockdown::SMC_LOCKDOWN),
        ),
        (
            "supermicro-ars121l-lockdown",
            AnyDriver::Lockdown(&lockdown::ARS121L_LOCKDOWN),
        ),
        (
            "ami-megarac-lockdown",
            AnyDriver::Lockdown(&lockdown::MEGARAC_LOCKDOWN),
        ),
        (
            "lenovo-ami-lockdown",
            AnyDriver::Lockdown(&lockdown::LenovoAmiLockdown),
        ),
        (
            "lenovo-gb300-lockdown",
            AnyDriver::Lockdown(&lockdown::GB300_LOCKDOWN),
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
            AnyDriver::Accounts(&accounts::ILO_ACCOUNTS),
        ),
        (
            "lenovo-xcc-accounts",
            AnyDriver::Accounts(&accounts::XCC_ACCOUNTS),
        ),
        (
            "ami-megarac-accounts",
            AnyDriver::Accounts(&accounts::MEGARAC_ACCOUNTS),
        ),
        (
            "nvidia-viking-accounts",
            AnyDriver::Accounts(&accounts::VIKING_ACCOUNTS),
        ),
        (
            "nvidia-openbmc-accounts",
            AnyDriver::Accounts(&accounts::OPENBMC_ACCOUNTS),
        ),
        (
            "nvidia-bluefield-accounts",
            AnyDriver::Accounts(&accounts::BLUEFIELD_ACCOUNTS),
        ),
        (
            "nvidia-switch-accounts",
            AnyDriver::Accounts(&accounts::SWITCH_ACCOUNTS),
        ),
        (
            "delta-power-shelf-accounts",
            AnyDriver::Accounts(&accounts::DELTA_POWER_SHELF_ACCOUNTS),
        ),
        (
            "liteon-power-shelf-accounts",
            AnyDriver::Accounts(&accounts::LITEON_POWER_SHELF_ACCOUNTS),
        ),
        (
            "dell-idrac-firmware",
            AnyDriver::Firmware(&firmware::IDRAC_FIRMWARE),
        ),
        (
            "ami-megarac-firmware",
            AnyDriver::Firmware(&firmware::MEGARAC_FIRMWARE),
        ),
        (
            "nvidia-viking-firmware",
            AnyDriver::Firmware(&firmware::VIKING_FIRMWARE),
        ),
        (
            "nvidia-openbmc-firmware",
            AnyDriver::Firmware(&firmware::OPENBMC_FIRMWARE),
        ),
        (
            "dell-idrac-boss-storage",
            AnyDriver::Storage(&storage::IdracBossStorage),
        ),
        ("nvidia-bluefield-dpu", AnyDriver::Dpu(&dpu::BlueField3Dpu)),
        ("nvidia-bluefield2-dpu", AnyDriver::Dpu(&dpu::BlueField2Dpu)),
        (
            "nvidia-hgx-attestation",
            AnyDriver::Attestation(&attestation::HGX_ATTESTATION),
        ),
        (
            "dell-idrac-console",
            AnyDriver::Console(&console::IdracConsole),
        ),
        ("hpe-ilo-console", AnyDriver::Console(&console::ILO_CONSOLE)),
        (
            "lenovo-xcc-console",
            AnyDriver::Console(&console::XCC_CONSOLE),
        ),
        (
            "supermicro-bmc-console",
            AnyDriver::Console(&console::SupermicroBmcConsole),
        ),
        (
            "ami-megarac-console",
            AnyDriver::Console(&console::MEGARAC_CONSOLE),
        ),
        (
            "lenovo-ami-console",
            AnyDriver::Console(&console::LENOVO_AMI_CONSOLE),
        ),
        (
            "lenovo-gb300-console",
            AnyDriver::Console(&console::GB300_CONSOLE),
        ),
        (
            "nvidia-viking-console",
            AnyDriver::Console(&console::VIKING_CONSOLE),
        ),
        (
            "nvidia-bluefield-console",
            AnyDriver::Console(&console::BLUEFIELD_CONSOLE),
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
