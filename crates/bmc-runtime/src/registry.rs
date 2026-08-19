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
use std::sync::Arc;

use bmc_platform::{
    Accounts, Attestation, Bios, BmcControl, BootOrder, Capability, CapabilitySelection, Console,
    Dpu, DriverId, Firmware, Lockdown, Power, SecureBoot, Storage,
};
use nv_redfish::Bmc;
use thiserror::Error;

/// Capability implementations supplied by one compiled-in driver.
pub struct DriverSet<B: Bmc> {
    power: Option<Arc<dyn Power<B>>>,
    bmc_control: Option<Arc<dyn BmcControl<B>>>,
    bios: Option<Arc<dyn Bios<B>>>,
    boot_order: Option<Arc<dyn BootOrder<B>>>,
    secure_boot: Option<Arc<dyn SecureBoot<B>>>,
    lockdown: Option<Arc<dyn Lockdown<B>>>,
    accounts: Option<Arc<dyn Accounts<B>>>,
    firmware: Option<Arc<dyn Firmware<B>>>,
    storage: Option<Arc<dyn Storage<B>>>,
    dpu: Option<Arc<dyn Dpu<B>>>,
    attestation: Option<Arc<dyn Attestation<B>>>,
    console: Option<Arc<dyn Console<B>>>,
}

impl<B: Bmc> DriverSet<B> {
    /// Creates an empty capability set.
    pub const fn new() -> Self {
        Self {
            power: None,
            bmc_control: None,
            bios: None,
            boot_order: None,
            secure_boot: None,
            lockdown: None,
            accounts: None,
            firmware: None,
            storage: None,
            dpu: None,
            attestation: None,
            console: None,
        }
    }

    /// Adds a host-power implementation.
    pub fn with_power(mut self, driver: Arc<dyn Power<B>>) -> Self {
        self.power = Some(driver);
        self
    }

    /// Adds a BMC-control implementation.
    pub fn with_bmc_control(mut self, driver: Arc<dyn BmcControl<B>>) -> Self {
        self.bmc_control = Some(driver);
        self
    }

    /// Adds a BIOS implementation.
    pub fn with_bios(mut self, driver: Arc<dyn Bios<B>>) -> Self {
        self.bios = Some(driver);
        self
    }

    /// Adds a boot-order implementation.
    pub fn with_boot_order(mut self, driver: Arc<dyn BootOrder<B>>) -> Self {
        self.boot_order = Some(driver);
        self
    }

    /// Adds a Secure Boot implementation.
    pub fn with_secure_boot(mut self, driver: Arc<dyn SecureBoot<B>>) -> Self {
        self.secure_boot = Some(driver);
        self
    }

    /// Adds a lockdown implementation.
    pub fn with_lockdown(mut self, driver: Arc<dyn Lockdown<B>>) -> Self {
        self.lockdown = Some(driver);
        self
    }

    /// Adds an account-management implementation.
    pub fn with_accounts(mut self, driver: Arc<dyn Accounts<B>>) -> Self {
        self.accounts = Some(driver);
        self
    }

    /// Adds a firmware implementation.
    pub fn with_firmware(mut self, driver: Arc<dyn Firmware<B>>) -> Self {
        self.firmware = Some(driver);
        self
    }

    /// Adds a storage implementation.
    pub fn with_storage(mut self, driver: Arc<dyn Storage<B>>) -> Self {
        self.storage = Some(driver);
        self
    }

    /// Adds a DPU implementation.
    pub fn with_dpu(mut self, driver: Arc<dyn Dpu<B>>) -> Self {
        self.dpu = Some(driver);
        self
    }

    /// Adds an attestation implementation.
    pub fn with_attestation(mut self, driver: Arc<dyn Attestation<B>>) -> Self {
        self.attestation = Some(driver);
        self
    }

    /// Adds a console implementation.
    pub fn with_console(mut self, driver: Arc<dyn Console<B>>) -> Self {
        self.console = Some(driver);
        self
    }
}

impl<B: Bmc> Default for DriverSet<B> {
    fn default() -> Self {
        Self::new()
    }
}

/// Injected standard and named capability implementations for transport `B`.
pub struct DriverRegistry<B: Bmc> {
    standard: DriverSet<B>,
    named: BTreeMap<DriverId, DriverSet<B>>,
}

impl<B: Bmc> DriverRegistry<B> {
    /// Creates a registry with the supplied standard Redfish implementations.
    pub const fn new(standard: DriverSet<B>) -> Self {
        Self {
            standard,
            named: BTreeMap::new(),
        }
    }

    /// Registers one named driver, rejecting duplicate identifiers.
    pub fn register(&mut self, id: DriverId, driver: DriverSet<B>) -> Result<(), RegistryError> {
        if self.named.contains_key(&id) {
            return Err(RegistryError::DuplicateDriver(id));
        }
        self.named.insert(id, driver);
        Ok(())
    }

    /// Registers one named driver and returns the updated registry.
    pub fn with_driver(
        mut self,
        id: DriverId,
        driver: DriverSet<B>,
    ) -> Result<Self, RegistryError> {
        self.register(id, driver)?;
        Ok(self)
    }

    /// Resolves a host-power implementation.
    pub fn power(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Power<B>>, DispatchError> {
        self.resolve(Capability::Power, selection, |set| &set.power)
    }

    /// Resolves a BMC-control implementation.
    pub fn bmc_control(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn BmcControl<B>>, DispatchError> {
        self.resolve(Capability::BmcControl, selection, |set| &set.bmc_control)
    }

    /// Resolves a BIOS implementation.
    pub fn bios(&self, selection: &CapabilitySelection) -> Result<Arc<dyn Bios<B>>, DispatchError> {
        self.resolve(Capability::Bios, selection, |set| &set.bios)
    }

    /// Resolves a boot-order implementation.
    pub fn boot_order(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn BootOrder<B>>, DispatchError> {
        self.resolve(Capability::BootOrder, selection, |set| &set.boot_order)
    }

    /// Resolves a Secure Boot implementation.
    pub fn secure_boot(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn SecureBoot<B>>, DispatchError> {
        self.resolve(Capability::SecureBoot, selection, |set| &set.secure_boot)
    }

    /// Resolves a lockdown implementation.
    pub fn lockdown(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Lockdown<B>>, DispatchError> {
        self.resolve(Capability::Lockdown, selection, |set| &set.lockdown)
    }

    /// Resolves an account-management implementation.
    pub fn accounts(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Accounts<B>>, DispatchError> {
        self.resolve(Capability::Accounts, selection, |set| &set.accounts)
    }

    /// Resolves a firmware implementation.
    pub fn firmware(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Firmware<B>>, DispatchError> {
        self.resolve(Capability::Firmware, selection, |set| &set.firmware)
    }

    /// Resolves a storage implementation.
    pub fn storage(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Storage<B>>, DispatchError> {
        self.resolve(Capability::Storage, selection, |set| &set.storage)
    }

    /// Resolves a DPU implementation.
    pub fn dpu(&self, selection: &CapabilitySelection) -> Result<Arc<dyn Dpu<B>>, DispatchError> {
        self.resolve(Capability::Dpu, selection, |set| &set.dpu)
    }

    /// Resolves an attestation implementation.
    pub fn attestation(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Attestation<B>>, DispatchError> {
        self.resolve(Capability::Attestation, selection, |set| &set.attestation)
    }

    /// Resolves a console implementation.
    pub fn console(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<Arc<dyn Console<B>>, DispatchError> {
        self.resolve(Capability::Console, selection, |set| &set.console)
    }

    fn resolve<T: ?Sized>(
        &self,
        capability: Capability,
        selection: &CapabilitySelection,
        get: impl Fn(&DriverSet<B>) -> &Option<Arc<T>>,
    ) -> Result<Arc<T>, DispatchError> {
        match selection {
            CapabilitySelection::Unsupported => Err(DispatchError::Unsupported(capability)),
            CapabilitySelection::Standard => {
                get(&self.standard)
                    .clone()
                    .ok_or(DispatchError::CapabilityNotImplemented {
                        capability,
                        driver: None,
                    })
            }
            CapabilitySelection::Driver(id) => {
                let set = self
                    .named
                    .get(id)
                    .ok_or_else(|| DispatchError::DriverNotRegistered {
                        capability,
                        driver: id.clone(),
                    })?;
                get(set)
                    .clone()
                    .ok_or_else(|| DispatchError::CapabilityNotImplemented {
                        capability,
                        driver: Some(id.clone()),
                    })
            }
        }
    }
}

/// Failure while constructing a driver registry.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RegistryError {
    /// A named driver identifier was registered more than once.
    #[error("driver {0} is already registered")]
    DuplicateDriver(DriverId),
}

/// Failure to resolve a selected capability implementation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DispatchError {
    /// The persisted driver map marks the capability unsupported.
    #[error("{0} is unsupported for this BMC")]
    Unsupported(Capability),
    /// The driver map names a driver absent from this process.
    #[error("driver {driver} selected for {capability} is not registered")]
    DriverNotRegistered {
        /// Capability being resolved.
        capability: Capability,
        /// Missing named driver.
        driver: DriverId,
    },
    /// The selected driver does not supply the requested capability.
    #[error("selected driver {driver:?} does not implement {capability}")]
    CapabilityNotImplemented {
        /// Capability being resolved.
        capability: Capability,
        /// Named driver, or `None` for the standard implementation.
        driver: Option<DriverId>,
    },
}
