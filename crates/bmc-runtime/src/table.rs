/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::collections::BTreeMap;

use bmc_platform::{
    Accounts, Attestation, Bios, BmcControl, BootOrder, Capability, CapabilitySelection, Console,
    Dpu, DriverId, DriverMap, Firmware, Lockdown, Power, SecureBoot, Storage,
};
use nv_redfish::Bmc;
use thiserror::Error;

use crate::selection::RuleSet;

/// One compiled capability driver, as listed by the drivers crate.
pub enum AnyDriver<B: Bmc + 'static> {
    Power(&'static dyn Power<B>),
    BmcControl(&'static dyn BmcControl<B>),
    Bios(&'static dyn Bios<B>),
    BootOrder(&'static dyn BootOrder<B>),
    SecureBoot(&'static dyn SecureBoot<B>),
    Lockdown(&'static dyn Lockdown<B>),
    Accounts(&'static dyn Accounts<B>),
    Firmware(&'static dyn Firmware<B>),
    Storage(&'static dyn Storage<B>),
    Dpu(&'static dyn Dpu<B>),
    Attestation(&'static dyn Attestation<B>),
    Console(&'static dyn Console<B>),
}

/// The standard driver and named drivers for one capability.
struct Slot<T: ?Sized + 'static> {
    standard: Option<&'static T>,
    named: BTreeMap<DriverId, &'static T>,
}

impl<T: ?Sized + 'static> Default for Slot<T> {
    fn default() -> Self {
        Self {
            standard: None,
            named: BTreeMap::new(),
        }
    }
}

impl<T: ?Sized + 'static> Slot<T> {
    fn insert(&mut self, id: Option<DriverId>, driver: &'static T) -> Option<DriverId> {
        match id {
            None => self.standard.replace(driver).map(|_| standard_id()),
            Some(id) => self.named.insert(id.clone(), driver).map(|_| id),
        }
    }

    fn resolve(
        &self,
        capability: Capability,
        selection: &CapabilitySelection,
    ) -> Result<&T, DriverTableError> {
        match selection {
            CapabilitySelection::Unsupported => Err(DriverTableError::Unsupported { capability }),
            CapabilitySelection::Standard => self
                .standard
                .ok_or(DriverTableError::StandardNotImplemented { capability }),
            CapabilitySelection::Driver(driver) => {
                self.named
                    .get(driver)
                    .copied()
                    .ok_or_else(|| DriverTableError::UnknownDriver {
                        capability,
                        driver: driver.clone(),
                    })
            }
        }
    }

    fn check(
        &self,
        capability: Capability,
        selection: &CapabilitySelection,
    ) -> Result<(), DriverTableError> {
        match selection {
            CapabilitySelection::Unsupported => Ok(()),
            other => self.resolve(capability, other).map(drop),
        }
    }
}

fn standard_id() -> DriverId {
    "standard-driver"
        .parse()
        .expect("placeholder id for duplicate standard drivers is valid")
}

/// Every driver compiled in for one transport, keyed the way driver maps refer to them.
///
/// The table is the single source of truth for dispatch and for validating
/// persisted maps and selection rules.
pub struct DriverTable<B: Bmc + 'static> {
    power: Slot<dyn Power<B>>,
    bmc_control: Slot<dyn BmcControl<B>>,
    bios: Slot<dyn Bios<B>>,
    boot_order: Slot<dyn BootOrder<B>>,
    secure_boot: Slot<dyn SecureBoot<B>>,
    lockdown: Slot<dyn Lockdown<B>>,
    accounts: Slot<dyn Accounts<B>>,
    firmware: Slot<dyn Firmware<B>>,
    storage: Slot<dyn Storage<B>>,
    dpu: Slot<dyn Dpu<B>>,
    attestation: Slot<dyn Attestation<B>>,
    console: Slot<dyn Console<B>>,
}

impl<B: Bmc + 'static> Default for DriverTable<B> {
    fn default() -> Self {
        Self {
            power: Slot::default(),
            bmc_control: Slot::default(),
            bios: Slot::default(),
            boot_order: Slot::default(),
            secure_boot: Slot::default(),
            lockdown: Slot::default(),
            accounts: Slot::default(),
            firmware: Slot::default(),
            storage: Slot::default(),
            dpu: Slot::default(),
            attestation: Slot::default(),
            console: Slot::default(),
        }
    }
}

impl<B: Bmc + 'static> DriverTable<B> {
    /// Builds the table from one entry list; `None` ids are standard drivers.
    ///
    /// Rejects a capability whose standard driver, or an id, is listed twice.
    pub fn new(
        entries: impl IntoIterator<Item = (Option<DriverId>, AnyDriver<B>)>,
    ) -> Result<Self, DriverTableError> {
        let mut table = Self::default();
        for (id, driver) in entries {
            let capability = driver.capability();
            let duplicate = match driver {
                AnyDriver::Power(driver) => table.power.insert(id, driver),
                AnyDriver::BmcControl(driver) => table.bmc_control.insert(id, driver),
                AnyDriver::Bios(driver) => table.bios.insert(id, driver),
                AnyDriver::BootOrder(driver) => table.boot_order.insert(id, driver),
                AnyDriver::SecureBoot(driver) => table.secure_boot.insert(id, driver),
                AnyDriver::Lockdown(driver) => table.lockdown.insert(id, driver),
                AnyDriver::Accounts(driver) => table.accounts.insert(id, driver),
                AnyDriver::Firmware(driver) => table.firmware.insert(id, driver),
                AnyDriver::Storage(driver) => table.storage.insert(id, driver),
                AnyDriver::Dpu(driver) => table.dpu.insert(id, driver),
                AnyDriver::Attestation(driver) => table.attestation.insert(id, driver),
                AnyDriver::Console(driver) => table.console.insert(id, driver),
            };
            if let Some(driver) = duplicate {
                return Err(DriverTableError::Duplicate { capability, driver });
            }
        }
        Ok(table)
    }

    /// The map a BMC gets when no rule matches: `Standard` for every capability
    /// with a compiled standard driver, `Unsupported` otherwise.
    pub fn default_map(&self) -> DriverMap {
        let mut map = DriverMap::filled(CapabilitySelection::Unsupported);
        for capability in Capability::ALL {
            if self
                .check(capability, &CapabilitySelection::Standard)
                .is_ok()
            {
                map.set(capability, CapabilitySelection::Standard);
            }
        }
        map
    }

    /// Checks that every selection in a persisted map names a compiled driver.
    pub fn validate_map(&self, map: &DriverMap) -> Result<(), DriverTableError> {
        map.iter()
            .try_for_each(|(capability, selection)| self.check(capability, selection))
    }

    /// Checks that every rule selects a compiled driver for its capability.
    pub fn validate_rules(&self, rules: &RuleSet) -> Result<(), DriverTableError> {
        rules
            .rules()
            .iter()
            .try_for_each(|rule| self.check(rule.capability, &rule.selection))
    }

    /// Fails before any I/O when `selection` is unsupported or names no compiled driver.
    pub fn check(
        &self,
        capability: Capability,
        selection: &CapabilitySelection,
    ) -> Result<(), DriverTableError> {
        match capability {
            Capability::Power => self.power.check(capability, selection),
            Capability::BmcControl => self.bmc_control.check(capability, selection),
            Capability::Bios => self.bios.check(capability, selection),
            Capability::BootOrder => self.boot_order.check(capability, selection),
            Capability::SecureBoot => self.secure_boot.check(capability, selection),
            Capability::Lockdown => self.lockdown.check(capability, selection),
            Capability::Accounts => self.accounts.check(capability, selection),
            Capability::Firmware => self.firmware.check(capability, selection),
            Capability::Storage => self.storage.check(capability, selection),
            Capability::Dpu => self.dpu.check(capability, selection),
            Capability::Attestation => self.attestation.check(capability, selection),
            Capability::Console => self.console.check(capability, selection),
        }
    }

    pub fn power(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Power<B>, DriverTableError> {
        self.power.resolve(Capability::Power, selection)
    }

    pub fn bmc_control(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn BmcControl<B>, DriverTableError> {
        self.bmc_control.resolve(Capability::BmcControl, selection)
    }

    pub fn bios(&self, selection: &CapabilitySelection) -> Result<&dyn Bios<B>, DriverTableError> {
        self.bios.resolve(Capability::Bios, selection)
    }

    pub fn boot_order(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn BootOrder<B>, DriverTableError> {
        self.boot_order.resolve(Capability::BootOrder, selection)
    }

    pub fn secure_boot(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn SecureBoot<B>, DriverTableError> {
        self.secure_boot.resolve(Capability::SecureBoot, selection)
    }

    pub fn lockdown(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Lockdown<B>, DriverTableError> {
        self.lockdown.resolve(Capability::Lockdown, selection)
    }

    pub fn accounts(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Accounts<B>, DriverTableError> {
        self.accounts.resolve(Capability::Accounts, selection)
    }

    pub fn firmware(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Firmware<B>, DriverTableError> {
        self.firmware.resolve(Capability::Firmware, selection)
    }

    pub fn storage(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Storage<B>, DriverTableError> {
        self.storage.resolve(Capability::Storage, selection)
    }

    pub fn dpu(&self, selection: &CapabilitySelection) -> Result<&dyn Dpu<B>, DriverTableError> {
        self.dpu.resolve(Capability::Dpu, selection)
    }

    pub fn attestation(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Attestation<B>, DriverTableError> {
        self.attestation.resolve(Capability::Attestation, selection)
    }

    pub fn console(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Console<B>, DriverTableError> {
        self.console.resolve(Capability::Console, selection)
    }
}

impl<B: Bmc + 'static> AnyDriver<B> {
    /// The capability this driver implements.
    pub const fn capability(&self) -> Capability {
        match self {
            Self::Power(_) => Capability::Power,
            Self::BmcControl(_) => Capability::BmcControl,
            Self::Bios(_) => Capability::Bios,
            Self::BootOrder(_) => Capability::BootOrder,
            Self::SecureBoot(_) => Capability::SecureBoot,
            Self::Lockdown(_) => Capability::Lockdown,
            Self::Accounts(_) => Capability::Accounts,
            Self::Firmware(_) => Capability::Firmware,
            Self::Storage(_) => Capability::Storage,
            Self::Dpu(_) => Capability::Dpu,
            Self::Attestation(_) => Capability::Attestation,
            Self::Console(_) => Capability::Console,
        }
    }
}

/// Failure to build or consult the compiled driver table.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DriverTableError {
    /// The persisted map marks this capability unsupported on this BMC.
    #[error("{capability} is unsupported on this BMC")]
    Unsupported { capability: Capability },
    /// No standard driver is compiled for this capability.
    #[error("no standard {capability} driver is compiled in")]
    StandardNotImplemented { capability: Capability },
    /// The map or rule names a driver that is not compiled in for this capability.
    #[error("unknown {capability} driver {driver}")]
    UnknownDriver {
        capability: Capability,
        driver: DriverId,
    },
    /// The entry list registered the same driver twice.
    #[error("{capability} driver {driver} is registered twice")]
    Duplicate {
        capability: Capability,
        driver: DriverId,
    },
}
