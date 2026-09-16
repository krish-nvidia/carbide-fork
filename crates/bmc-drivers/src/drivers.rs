/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! The compiled driver catalogue: every driver NICo carries, keyed by a typed
//! [`Driver`], and the dispatch from a persisted selection to the driver.
//!
//! Adding a driver is one line in [`drivers!`] below plus its own file under
//! the capability module. The macro derives the wire id, the capability, and
//! the dispatch arm, so a driver named in a rule for the wrong capability, or
//! missing from the list, fails to compile or fails table validation.

use std::fmt;
use std::str::FromStr;

use bmc_platform::{
    Accounts, Attestation, Bios, BmcControl, BootOrder, Capability, Console, Dpu, Firmware,
    Lockdown, Power, SecureBoot, Storage,
};
use nv_redfish::core::Bmc;
use thiserror::Error;

use crate::selection::{CapabilitySelection, DriverMap, Rules};
use crate::{
    accounts, attestation, bios, bmc_control, boot_order, console, dpu, firmware, lockdown, power,
    secure_boot, storage,
};

/// Declares the catalogue: one block per capability naming its trait, the
/// [`Catalog`] accessor, optionally its standard driver, and every named
/// driver as `Variant = "wire-id" => path::Type`.
macro_rules! drivers {
    ($(
        $capability:ident as $accessor:ident : $trait:ident $(= $standard:path)? {
            $( $variant:ident = $id:literal => $driver:path, )*
        }
    )*) => {
        /// A compiled capability driver, or a plugin this binary does not carry.
        ///
        /// Known variants serialize as their kebab-case wire id, which is what
        /// selection rules and persisted driver maps store.
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum Driver {
            $($(
                #[doc = concat!("`", $id, "`")]
                $variant,
            )*)*
            /// A well-formed driver id this binary does not compile in.
            Plugin(PluginId),
        }

        impl Driver {
            /// Every compiled driver.
            pub const ALL: &'static [Self] = &[$($( Self::$variant, )*)*];

            /// The capability this driver implements; `None` for a plugin.
            pub const fn capability(&self) -> Option<Capability> {
                match self {
                    $($( Self::$variant => Some(Capability::$capability), )*)*
                    Self::Plugin(_) => None,
                }
            }

            /// The id as written in rules and driver maps.
            pub fn as_str(&self) -> &str {
                match self {
                    $($( Self::$variant => $id, )*)*
                    Self::Plugin(id) => id.as_str(),
                }
            }
        }

        impl FromStr for Driver {
            type Err = PluginIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($( $id => Ok(Self::$variant), )*)*
                    other => other.parse().map(Self::Plugin),
                }
            }
        }

        impl Drivers {
            /// The map a BMC gets when no rule matches: `Standard` for every
            /// capability with a compiled standard driver, `Unsupported` otherwise.
            pub fn default_map() -> DriverMap {
                let mut map = DriverMap::filled(CapabilitySelection::Unsupported);
                $(
                    if has_standard!($($standard)?) {
                        map.set(Capability::$capability, CapabilitySelection::Standard);
                    }
                )*
                map
            }
        }

        impl<B: Bmc + 'static> Catalog<B> for Drivers {
            $(
                fn $accessor(
                    &self,
                    selection: &CapabilitySelection,
                ) -> Result<&dyn $trait<B>, CatalogError> {
                    let capability = Capability::$capability;
                    match selection {
                        CapabilitySelection::Unsupported => {
                            Err(CatalogError::Unsupported { capability })
                        }
                        CapabilitySelection::Standard => {
                            standard_driver!(capability, $($standard)?)
                        }
                        $( CapabilitySelection::Driver(Driver::$variant) => Ok(&$driver), )*
                        CapabilitySelection::Driver(Driver::Plugin(id)) => {
                            Err(CatalogError::UnknownPlugin { capability, id: id.clone() })
                        }
                        CapabilitySelection::Driver(other) => Err(CatalogError::WrongCapability {
                            capability,
                            driver: other.clone(),
                        }),
                    }
                }
            )*
        }
    };
}

macro_rules! has_standard {
    ($standard:path) => {
        true
    };
    () => {
        false
    };
}

macro_rules! standard_driver {
    ($capability:expr, $standard:path) => {
        Ok(&$standard)
    };
    ($capability:expr,) => {
        Err(CatalogError::StandardNotImplemented {
            capability: $capability,
        })
    };
}

drivers! {
    Power as power: Power = power::StandardPower {
        DellIdracPower = "dell-idrac-power" => power::IdracPower,
        DeltaPowerShelfPower = "delta-power-shelf-power" => power::DeltaPowerShelfPower,
        HpeIloPower = "hpe-ilo-power" => power::IloPower,
        LenovoSr650V4Power = "lenovo-sr650-v4-power" => power::Sr650V4Power,
        LenovoSr675V3OvxPower = "lenovo-sr675-v3-ovx-power" => power::Sr675V3OvxPower,
        LenovoXccPower = "lenovo-xcc-power" => power::XccPower,
        LiteOnPowerShelfPower = "liteon-power-shelf-power" => power::LiteOnPowerShelfPower,
        NvidiaOpenBmcPower = "nvidia-openbmc-power" => power::OpenBmcPower,
        NvidiaVikingPower = "nvidia-viking-power" => power::VikingPower,
        SupermicroSmcPower = "supermicro-smc-power" => power::SmcPower,
    }
    BmcControl as bmc_control: BmcControl = bmc_control::StandardBmcControl {
        AmiMegaRacBmcControl = "ami-megarac-bmc-control" => bmc_control::MegaRacBmcControl,
        DellIdracBmcControl = "dell-idrac-bmc-control" => bmc_control::IdracBmcControl,
        HpeIloBmcControl = "hpe-ilo-bmc-control" => bmc_control::IloBmcControl,
        SupermicroSmcBmcControl = "supermicro-smc-bmc-control" => bmc_control::SmcBmcControl,
    }
    Bios as bios: Bios = bios::StandardBios {
        AmiMegaRacBios = "ami-megarac-bios" => bios::MegaRacBios,
        DellIdracBios = "dell-idrac-bios" => bios::IdracBios,
        LenovoXccBios = "lenovo-xcc-bios" => bios::XccBios,
        NvidiaBlueFieldBios = "nvidia-bluefield-bios" => bios::BlueFieldBios,
        NvidiaOpenBmcBios = "nvidia-openbmc-bios" => bios::OpenBmcBios,
        NvidiaVikingBios = "nvidia-viking-bios" => bios::VikingBios,
    }
    BootOrder as boot_order: BootOrder = boot_order::StandardBootOrder {
        DellIdracBootOrder = "dell-idrac-boot-order" => boot_order::IdracBootOrder,
        HpeIloBootOrder = "hpe-ilo-boot-order" => boot_order::IloBootOrder,
        NvidiaOpenBmcBootOrder = "nvidia-openbmc-boot-order" => boot_order::OpenBmcBootOrder,
        SupermicroX13BootOrder = "supermicro-x13-boot-order" => boot_order::X13BootOrder,
    }
    SecureBoot as secure_boot: SecureBoot = secure_boot::StandardSecureBoot {
    }
    Lockdown as lockdown: Lockdown {
        AmiMegaRacLockdown = "ami-megarac-lockdown" => lockdown::MegaRacLockdown,
        DellIdracLockdown = "dell-idrac-lockdown" => lockdown::IdracLockdown,
        HpeIloLockdown = "hpe-ilo-lockdown" => lockdown::IloLockdown,
        LenovoAmiLockdown = "lenovo-ami-lockdown" => lockdown::LenovoAmiLockdown,
        LenovoGb300Lockdown = "lenovo-gb300-lockdown" => lockdown::Gb300Lockdown,
        LenovoXccLockdown = "lenovo-xcc-lockdown" => lockdown::XccLockdown,
        NvidiaOpenBmcLockdown = "nvidia-openbmc-lockdown" => lockdown::OpenBmcLockdown,
        NvidiaVikingLockdown = "nvidia-viking-lockdown" => lockdown::VikingLockdown,
        SupermicroArs121lLockdown = "supermicro-ars121l-lockdown" => lockdown::Ars121lLockdown,
        SupermicroSmcLockdown = "supermicro-smc-lockdown" => lockdown::SmcLockdown,
    }
    Accounts as accounts: Accounts = accounts::StandardAccounts {
        DellIdracAccounts = "dell-idrac-accounts" => accounts::IdracAccounts,
        DeltaPowerShelfAccounts = "delta-power-shelf-accounts" => accounts::DeltaPowerShelfAccounts,
        HpeIloAccounts = "hpe-ilo-accounts" => accounts::IloAccounts,
        LenovoXccAccounts = "lenovo-xcc-accounts" => accounts::XccAccounts,
        LiteOnPowerShelfAccounts = "liteon-power-shelf-accounts" => accounts::LiteOnPowerShelfAccounts,
        NvidiaBlueFieldAccounts = "nvidia-bluefield-accounts" => accounts::BlueFieldAccounts,
        NvidiaOpenBmcAccounts = "nvidia-openbmc-accounts" => accounts::OpenBmcAccounts,
        NvidiaSwitchAccounts = "nvidia-switch-accounts" => accounts::SwitchAccounts,
        NvidiaVikingAccounts = "nvidia-viking-accounts" => accounts::VikingAccounts,
    }
    Firmware as firmware: Firmware = firmware::StandardFirmware {
        AmiMegaRacFirmware = "ami-megarac-firmware" => firmware::MegaRacFirmware,
        DellIdracFirmware = "dell-idrac-firmware" => firmware::IdracFirmware,
        NvidiaOpenBmcFirmware = "nvidia-openbmc-firmware" => firmware::OpenBmcFirmware,
        NvidiaVikingFirmware = "nvidia-viking-firmware" => firmware::VikingFirmware,
    }
    Storage as storage: Storage {
        DellIdracBossStorage = "dell-idrac-boss-storage" => storage::IdracBossStorage,
    }
    Dpu as dpu: Dpu {
        NvidiaBlueField2Dpu = "nvidia-bluefield2-dpu" => dpu::BlueField2Dpu,
        NvidiaBlueFieldDpu = "nvidia-bluefield-dpu" => dpu::BlueField3Dpu,
    }
    Attestation as attestation: Attestation = attestation::StandardAttestation {
        NvidiaHgxAttestation = "nvidia-hgx-attestation" => attestation::HgxAttestation,
    }
    Console as console: Console {
        AmiMegaRacConsole = "ami-megarac-console" => console::MegaRacConsole,
        DellIdracConsole = "dell-idrac-console" => console::IdracConsole,
        HpeIloConsole = "hpe-ilo-console" => console::IloConsole,
        LenovoAmiConsole = "lenovo-ami-console" => console::LenovoAmiConsole,
        LenovoGb300Console = "lenovo-gb300-console" => console::Gb300Console,
        LenovoXccConsole = "lenovo-xcc-console" => console::XccConsole,
        NvidiaBlueFieldConsole = "nvidia-bluefield-console" => console::BlueFieldConsole,
        NvidiaVikingConsole = "nvidia-viking-console" => console::VikingConsole,
        SupermicroBmcConsole = "supermicro-bmc-console" => console::SupermicroBmcConsole,
    }
}

impl fmt::Display for Driver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A well-formed driver id that names no compiled driver.
///
/// Persisted maps and deployment overrides may carry one; validation
/// rejects it before any I/O so a pluggable driver mechanism can adopt the
/// same wire form later.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginId(String);

impl PluginId {
    /// Returns the id as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for PluginId {
    type Err = PluginIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            return Err(PluginIdError::Empty);
        }
        if matches!(value, "standard" | "unsupported") {
            return Err(PluginIdError::Reserved);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(PluginIdError::InvalidCharacter);
        }
        if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
            return Err(PluginIdError::InvalidSeparator);
        }
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PluginIdError {
    #[error("driver id is empty")]
    Empty,
    #[error("driver id is reserved for a capability-selection sentinel")]
    Reserved,
    #[error("driver id must contain only lowercase ASCII letters, digits, and hyphens")]
    InvalidCharacter,
    #[error("driver id must use single hyphens between nonempty segments")]
    InvalidSeparator,
}

/// Resolves a persisted selection to the driver that serves it.
///
/// [`Drivers`] is the compiled catalogue; test doubles implement only the
/// accessors they exercise and inherit `Unsupported` for the rest.
pub trait Catalog<B: Bmc>: Send + Sync {
    fn power(&self, selection: &CapabilitySelection) -> Result<&dyn Power<B>, CatalogError> {
        unsupported(Capability::Power, selection)
    }

    fn bmc_control(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn BmcControl<B>, CatalogError> {
        unsupported(Capability::BmcControl, selection)
    }

    fn bios(&self, selection: &CapabilitySelection) -> Result<&dyn Bios<B>, CatalogError> {
        unsupported(Capability::Bios, selection)
    }

    fn boot_order(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn BootOrder<B>, CatalogError> {
        unsupported(Capability::BootOrder, selection)
    }

    fn secure_boot(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn SecureBoot<B>, CatalogError> {
        unsupported(Capability::SecureBoot, selection)
    }

    fn lockdown(&self, selection: &CapabilitySelection) -> Result<&dyn Lockdown<B>, CatalogError> {
        unsupported(Capability::Lockdown, selection)
    }

    fn accounts(&self, selection: &CapabilitySelection) -> Result<&dyn Accounts<B>, CatalogError> {
        unsupported(Capability::Accounts, selection)
    }

    fn firmware(&self, selection: &CapabilitySelection) -> Result<&dyn Firmware<B>, CatalogError> {
        unsupported(Capability::Firmware, selection)
    }

    fn storage(&self, selection: &CapabilitySelection) -> Result<&dyn Storage<B>, CatalogError> {
        unsupported(Capability::Storage, selection)
    }

    fn dpu(&self, selection: &CapabilitySelection) -> Result<&dyn Dpu<B>, CatalogError> {
        unsupported(Capability::Dpu, selection)
    }

    fn attestation(
        &self,
        selection: &CapabilitySelection,
    ) -> Result<&dyn Attestation<B>, CatalogError> {
        unsupported(Capability::Attestation, selection)
    }

    fn console(&self, selection: &CapabilitySelection) -> Result<&dyn Console<B>, CatalogError> {
        unsupported(Capability::Console, selection)
    }
}

fn unsupported<'a, T: ?Sized>(
    capability: Capability,
    _selection: &CapabilitySelection,
) -> Result<&'a T, CatalogError> {
    Err(CatalogError::Unsupported { capability })
}

/// The catalogue compiled into this binary.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drivers;

impl Drivers {
    /// Fails before any I/O when `selection` names no compiled driver for `capability`.
    pub fn check(
        capability: Capability,
        selection: &CapabilitySelection,
    ) -> Result<(), CatalogError> {
        match selection {
            CapabilitySelection::Unsupported => Ok(()),
            CapabilitySelection::Standard => {
                if Self::default_map().get(capability) == &CapabilitySelection::Standard {
                    Ok(())
                } else {
                    Err(CatalogError::StandardNotImplemented { capability })
                }
            }
            CapabilitySelection::Driver(Driver::Plugin(id)) => Err(CatalogError::UnknownPlugin {
                capability,
                id: id.clone(),
            }),
            CapabilitySelection::Driver(driver) if driver.capability() == Some(capability) => {
                Ok(())
            }
            CapabilitySelection::Driver(driver) => Err(CatalogError::WrongCapability {
                capability,
                driver: driver.clone(),
            }),
        }
    }

    /// Checks that every selection in a persisted map names a compiled driver.
    pub fn validate_map(map: &DriverMap) -> Result<(), CatalogError> {
        map.iter()
            .try_for_each(|(capability, selection)| Self::check(capability, selection))
    }

    /// Checks that every rule selects compiled drivers for their capabilities.
    pub fn validate_rules(rules: &Rules) -> Result<(), CatalogError> {
        rules.rules().iter().try_for_each(|rule| {
            rule.selections
                .iter()
                .try_for_each(|(capability, selection)| Self::check(*capability, selection))
        })
    }
}

/// Failure to resolve a selection against the compiled catalogue.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogError {
    /// The persisted map marks this capability unsupported on this BMC.
    #[error("{capability} is unsupported on this BMC")]
    Unsupported { capability: Capability },
    /// No standard driver is compiled for this capability.
    #[error("no standard {capability} driver is compiled in")]
    StandardNotImplemented { capability: Capability },
    /// The selection names a driver of a different capability.
    #[error("{driver} is not a {capability} driver")]
    WrongCapability {
        capability: Capability,
        driver: Driver,
    },
    /// The selection names a driver this binary does not compile in.
    #[error("unknown {capability} driver {id}")]
    UnknownPlugin {
        capability: Capability,
        id: PluginId,
    },
}

#[cfg(test)]
mod tests {
    use bmc_mock::test_support::TestBmc;

    use super::*;
    use crate::rules::built_in_rules;

    #[test]
    fn every_rule_names_compiled_drivers_of_the_right_capability() {
        Drivers::validate_rules(&built_in_rules()).expect("built-in rules are consistent");
        for driver in Driver::ALL {
            assert_eq!(
                driver.as_str().parse::<Driver>().as_ref(),
                Ok(driver),
                "{driver} round-trips through its wire id"
            );
        }
    }

    #[test]
    fn selections_resolve_or_fail_before_io() {
        let catalog: &dyn Catalog<TestBmc> = &Drivers;
        assert!(catalog.power(&CapabilitySelection::Standard).is_ok());
        assert_eq!(
            Drivers::check(Capability::Storage, &CapabilitySelection::Standard),
            Err(CatalogError::StandardNotImplemented {
                capability: Capability::Storage,
            })
        );
        assert_eq!(
            catalog.power(&CapabilitySelection::Unsupported).err(),
            Some(CatalogError::Unsupported {
                capability: Capability::Power,
            })
        );
        assert_eq!(
            catalog
                .power(&CapabilitySelection::Driver(Driver::DellIdracBios))
                .err(),
            Some(CatalogError::WrongCapability {
                capability: Capability::Power,
                driver: Driver::DellIdracBios,
            })
        );
        let plugin: Driver = "acme-power".parse().expect("well-formed plugin id");
        assert!(matches!(plugin, Driver::Plugin(_)));
        assert!(matches!(
            catalog.power(&CapabilitySelection::Driver(plugin)),
            Err(CatalogError::UnknownPlugin { .. })
        ));
        assert!("Bad Id".parse::<Driver>().is_err());
    }
}
