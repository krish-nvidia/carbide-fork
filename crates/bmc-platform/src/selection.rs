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

use std::fmt;
use std::str::FromStr;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A BMC capability with its own driver trait and driver-map slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    Power,
    BmcControl,
    Bios,
    BootOrder,
    SecureBoot,
    Lockdown,
    Accounts,
    Firmware,
    Storage,
    Dpu,
    Attestation,
    Console,
}

impl Capability {
    /// Every capability in stable wire order; also the [`DriverMap`] slot order.
    pub const ALL: [Self; 12] = [
        Self::Power,
        Self::BmcControl,
        Self::Bios,
        Self::BootOrder,
        Self::SecureBoot,
        Self::Lockdown,
        Self::Accounts,
        Self::Firmware,
        Self::Storage,
        Self::Dpu,
        Self::Attestation,
        Self::Console,
    ];

    /// The single source of each capability's wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::BmcControl => "bmc_control",
            Self::Bios => "bios",
            Self::BootOrder => "boot_order",
            Self::SecureBoot => "secure_boot",
            Self::Lockdown => "lockdown",
            Self::Accounts => "accounts",
            Self::Firmware => "firmware",
            Self::Storage => "storage",
            Self::Dpu => "dpu",
            Self::Attestation => "attestation",
            Self::Console => "console",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Capability {
    type Err = UnknownCapability;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == value)
            .ok_or_else(|| UnknownCapability(value.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("unknown capability {0:?}")]
pub struct UnknownCapability(String);

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Identifier of a named (non-standard) capability driver.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct DriverId(String);

impl DriverId {
    /// Returns the id as written in rules and driver maps.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DriverId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for DriverId {
    type Err = DriverIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            return Err(DriverIdError::Empty);
        }
        if matches!(value, "standard" | "unsupported") {
            return Err(DriverIdError::Reserved);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(DriverIdError::InvalidCharacter);
        }
        if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
            return Err(DriverIdError::InvalidSeparator);
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for DriverId {
    type Error = DriverIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DriverIdError {
    #[error("driver id is empty")]
    Empty,
    #[error("driver id is reserved for a capability-selection sentinel")]
    Reserved,
    #[error("driver id must contain only lowercase ASCII letters, digits, and hyphens")]
    InvalidCharacter,
    #[error("driver id must use single hyphens between nonempty segments")]
    InvalidSeparator,
}

/// Which driver serves one capability on one BMC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilitySelection {
    Standard,
    Unsupported,
    Driver(DriverId),
}

impl Serialize for CapabilitySelection {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Standard => serializer.serialize_str("standard"),
            Self::Unsupported => serializer.serialize_str("unsupported"),
            Self::Driver(driver) => driver.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for CapabilitySelection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "standard" => Ok(Self::Standard),
            "unsupported" => Ok(Self::Unsupported),
            _ => value
                .parse()
                .map(Self::Driver)
                .map_err(serde::de::Error::custom),
        }
    }
}

/// The complete per-capability driver selection persisted for one BMC.
///
/// Serializes as an object with one key per capability, in
/// [`Capability::ALL`] order; deserialization requires every key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriverMap([CapabilitySelection; 12]);

impl DriverMap {
    /// A map that selects `selection` for every capability.
    pub fn filled(selection: CapabilitySelection) -> Self {
        Self(std::array::from_fn(|_| selection.clone()))
    }

    /// Returns the selection for `capability`.
    pub fn get(&self, capability: Capability) -> &CapabilitySelection {
        &self.0[capability.index()]
    }

    /// Replaces the selection for `capability`.
    pub fn set(&mut self, capability: Capability, selection: CapabilitySelection) {
        self.0[capability.index()] = selection;
    }

    /// Returns `self` with `capability` switched to `selection`.
    pub fn with(mut self, capability: Capability, selection: CapabilitySelection) -> Self {
        self.set(capability, selection);
        self
    }

    /// Iterates selections in [`Capability::ALL`] order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (Capability, &CapabilitySelection)> {
        Capability::ALL.into_iter().zip(&self.0)
    }
}

impl Serialize for DriverMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (capability, selection) in self.iter() {
            map.serialize_entry(capability.as_str(), selection)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for DriverMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor;

        impl<'de> Visitor<'de> for MapVisitor {
            type Value = DriverMap;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with one selection per capability")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut slots: [Option<CapabilitySelection>; 12] = Default::default();
                while let Some((capability, selection)) =
                    access.next_entry::<Capability, CapabilitySelection>()?
                {
                    if slots[capability.index()].replace(selection).is_some() {
                        return Err(serde::de::Error::duplicate_field(capability.as_str()));
                    }
                }
                let mut selections = Vec::with_capacity(slots.len());
                for (capability, slot) in Capability::ALL.into_iter().zip(slots) {
                    selections.push(
                        slot.ok_or_else(|| serde::de::Error::missing_field(capability.as_str()))?,
                    );
                }
                selections
                    .try_into()
                    .map(DriverMap)
                    .map_err(|_| serde::de::Error::custom("driver map has the wrong arity"))
            }
        }

        deserializer.deserialize_map(MapVisitor)
    }
}

#[cfg(test)]
mod tests {

    use carbide_test_support::Outcome::{Fails, Yields};
    use carbide_test_support::{scenarios, value_scenarios};
    use serde_json::json;

    use super::*;

    fn complete_map() -> DriverMap {
        DriverMap::filled(CapabilitySelection::Standard)
            .with(Capability::Storage, CapabilitySelection::Unsupported)
            .with(Capability::Dpu, CapabilitySelection::Unsupported)
            .with(
                Capability::Console,
                CapabilitySelection::Driver(
                    "xcc-console".parse().expect("fixture driver id is valid"),
                ),
            )
    }

    #[test]
    fn driver_ids_enforce_canonical_form() {
        scenarios!(run = |value: &str| value.parse::<DriverId>().map(|id| id.to_string());
            "valid ids" {
                "redfish-standard" => Yields("redfish-standard".to_string()),
                "sr650v4-power" => Yields("sr650v4-power".to_string()),
                "driver2" => Yields("driver2".to_string()),
            }
            "invalid ids" {
                "" => Fails,
                "standard" => Fails,
                "unsupported" => Fails,
                "XCC" => Fails,
                "xcc_power" => Fails,
                "-xcc" => Fails,
                "xcc-" => Fails,
                "xcc--power" => Fails,
            }
        );
    }

    #[test]
    fn driver_selection_cannot_serialize_as_a_sentinel() {
        fn serialize_driver(value: &str) -> Result<serde_json::Value, DriverIdError> {
            let selection = CapabilitySelection::Driver(value.parse()?);
            Ok(serde_json::to_value(selection).expect("valid driver selection serializes"))
        }

        for sentinel in ["standard", "unsupported"] {
            assert_eq!(serialize_driver(sentinel), Err(DriverIdError::Reserved));
            assert!(!matches!(
                serde_json::from_value::<CapabilitySelection>(json!(sentinel))
                    .expect("sentinel selection deserializes"),
                CapabilitySelection::Driver(_)
            ));
        }
    }

    #[test]
    fn capabilities_have_stable_wire_names_and_order() {
        let names = Capability::ALL.map(|capability| capability.to_string());
        assert_eq!(
            names,
            [
                "power",
                "bmc_control",
                "bios",
                "boot_order",
                "secure_boot",
                "lockdown",
                "accounts",
                "firmware",
                "storage",
                "dpu",
                "attestation",
                "console",
            ]
        );
        for capability in Capability::ALL {
            let encoded = serde_json::to_string(&capability).expect("capability serializes");
            let decoded: Capability =
                serde_json::from_str(&encoded).expect("capability deserializes");
            assert_eq!(decoded, capability);
        }
    }

    #[test]
    fn capability_selections_use_plain_strings() {
        value_scenarios!(run = |selection: CapabilitySelection| serde_json::to_value(selection)
            .expect("selection serializes");
            "stable representation" {
                CapabilitySelection::Standard => json!("standard"),
                CapabilitySelection::Unsupported => json!("unsupported"),
                CapabilitySelection::Driver("xcc-console".parse().expect("valid id")) =>
                    json!("xcc-console"),
            }
        );
    }

    #[test]
    fn driver_map_requires_every_capability_and_round_trips() {
        let map = complete_map();
        let encoded = serde_json::to_value(&map).expect("driver map serializes");
        assert_eq!(
            serde_json::from_value::<DriverMap>(encoded.clone())
                .expect("complete map deserializes"),
            map
        );

        let mut missing = encoded;
        missing
            .as_object_mut()
            .expect("driver map is an object")
            .remove("console");
        assert!(serde_json::from_value::<DriverMap>(missing).is_err());
    }

    #[test]
    fn driver_map_access_is_complete_and_deterministic() {
        let mut map = complete_map();
        map.set(
            Capability::Storage,
            CapabilitySelection::Driver("xcc-storage".parse().expect("valid id")),
        );
        assert_eq!(
            map.get(Capability::Storage),
            &CapabilitySelection::Driver("xcc-storage".parse().expect("valid id"))
        );
        assert_eq!(
            map.iter()
                .map(|(capability, _)| capability)
                .collect::<Vec<_>>(),
            Capability::ALL
        );
    }
}
