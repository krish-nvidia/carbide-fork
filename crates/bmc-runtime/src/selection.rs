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

use blake3::{Hash, Hasher};
use bmc_platform::{CapabilitySelection, DriverMap, PlatformIdentity};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use version_compare::{Cmp, Version};

/// An identity value available to declarative selection rules.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityField {
    /// ServiceRoot `Vendor`.
    ServiceRootVendor,
    /// ServiceRoot `Product`.
    ServiceRootProduct,
    /// Any ServiceRoot OEM key.
    ServiceRootOemKey,
    /// Selected Manager model.
    ManagerModel,
    /// Selected Manager firmware version.
    ManagerFirmware,
    /// Selected ComputerSystem identifier.
    SystemId,
    /// Selected ComputerSystem manufacturer.
    SystemManufacturer,
    /// Selected ComputerSystem model.
    SystemModel,
    /// Selected ComputerSystem SKU.
    SystemSku,
    /// Selected ComputerSystem part number.
    SystemPartNumber,
    /// Selected ComputerSystem BIOS version.
    SystemBiosVersion,
    /// Any Chassis identifier.
    ChassisId,
    /// Any Chassis manufacturer.
    ChassisManufacturer,
    /// Any Chassis model.
    ChassisModel,
    /// Any Chassis part number.
    ChassisPartNumber,
}

/// Inclusive validated firmware-version bounds.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FirmwareVersionRange {
    minimum: String,
    maximum: String,
}

impl FirmwareVersionRange {
    /// Creates an inclusive range after validating both bounds and their order.
    pub fn new(minimum: String, maximum: String) -> Result<Self, FirmwareVersionRangeError> {
        let minimum_version =
            Version::from(&minimum).ok_or(FirmwareVersionRangeError::InvalidMinimum)?;
        let maximum_version =
            Version::from(&maximum).ok_or(FirmwareVersionRangeError::InvalidMaximum)?;
        if minimum_version.compare(maximum_version) == Cmp::Gt {
            return Err(FirmwareVersionRangeError::Reversed);
        }
        Ok(Self { minimum, maximum })
    }

    /// Returns the inclusive minimum version.
    pub fn minimum(&self) -> &str {
        &self.minimum
    }

    /// Returns the inclusive maximum version.
    pub fn maximum(&self) -> &str {
        &self.maximum
    }

    fn contains(&self, candidate: &str) -> bool {
        let Some(candidate) = Version::from(candidate) else {
            return false;
        };
        let (Some(minimum), Some(maximum)) =
            (Version::from(&self.minimum), Version::from(&self.maximum))
        else {
            return false;
        };
        candidate.compare(minimum) != Cmp::Lt && candidate.compare(maximum) != Cmp::Gt
    }
}

impl<'de> Deserialize<'de> for FirmwareVersionRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            minimum: String,
            maximum: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.minimum, wire.maximum).map_err(serde::de::Error::custom)
    }
}

/// Validation failure for inclusive firmware-version bounds.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FirmwareVersionRangeError {
    /// The minimum cannot be parsed by `version-compare`.
    #[error("minimum firmware version is invalid")]
    InvalidMinimum,
    /// The maximum cannot be parsed by `version-compare`.
    #[error("maximum firmware version is invalid")]
    InvalidMaximum,
    /// The minimum is greater than the maximum.
    #[error("minimum firmware version must not exceed maximum")]
    Reversed,
}

/// String comparison used by an identity matcher.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MatchPattern {
    /// Requires an exact case-sensitive value.
    Exact(String),
    /// Requires an exact ASCII case-insensitive value.
    ExactAsciiCaseInsensitive(String),
    /// Requires a case-sensitive prefix.
    Prefix(String),
    /// Requires a case-sensitive substring.
    Contains(String),
    /// Requires an exact match against one of several canonical values.
    OneOf(Vec<String>),
    /// Requires a parseable version within inclusive validated bounds.
    FirmwareVersionRange(FirmwareVersionRange),
}

impl MatchPattern {
    fn first_value(&self) -> Option<&str> {
        match self {
            Self::Exact(value)
            | Self::ExactAsciiCaseInsensitive(value)
            | Self::Prefix(value)
            | Self::Contains(value) => Some(value),
            Self::OneOf(values) => values.first().map(String::as_str),
            Self::FirmwareVersionRange(range) => Some(range.minimum()),
        }
    }

    fn matches(&self, candidate: &str) -> bool {
        match self {
            Self::Exact(value) => candidate == value,
            Self::ExactAsciiCaseInsensitive(value) => candidate.eq_ignore_ascii_case(value),
            Self::Prefix(value) => candidate.starts_with(value),
            Self::Contains(value) => candidate.contains(value),
            Self::OneOf(values) => values.iter().any(|value| candidate == value),
            Self::FirmwareVersionRange(range) => range.contains(candidate),
        }
    }

    fn hash_into(&self, hasher: &mut Hasher) {
        let (tag, value) = match self {
            Self::Exact(value) => (0, value),
            Self::ExactAsciiCaseInsensitive(value) => (1, value),
            Self::Prefix(value) => (2, value),
            Self::Contains(value) => (3, value),
            Self::OneOf(values) => {
                hasher.update(&[4]);
                hash_length(hasher, values.len());
                for value in values {
                    hash_string(hasher, value);
                }
                return;
            }
            Self::FirmwareVersionRange(range) => {
                hasher.update(&[5]);
                hash_string(hasher, range.minimum());
                hash_string(hasher, range.maximum());
                return;
            }
        };
        hasher.update(&[tag]);
        hash_string(hasher, value);
    }
}

/// One required identity predicate in a selection rule.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct IdentityMatcher {
    /// Identity field examined by this predicate.
    pub field: IdentityField,
    /// Comparison applied to every value for `field`.
    pub pattern: MatchPattern,
}

impl IdentityMatcher {
    /// Creates a field matcher.
    pub const fn new(field: IdentityField, pattern: MatchPattern) -> Self {
        Self { field, pattern }
    }

    /// Reports whether any value of this field satisfies the matcher.
    pub fn matches(&self, identity: &PlatformIdentity) -> bool {
        field_values(identity, self.field)
            .into_iter()
            .any(|value| self.pattern.matches(value))
    }
}

/// Explicit selection-rule precedence from broad defaults to local overrides.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum Precedence {
    /// Built-in standard/default behavior.
    StandardDefault,
    /// Vendor, manufacturer, or ServiceRoot OEM evidence.
    VendorManufacturer,
    /// BMC product or Manager identity evidence.
    BmcProductManager,
    /// Exact ComputerSystem model, SKU, or part-number evidence.
    ExactSystemIdentity,
    /// Deliberate deployment-local override.
    DeploymentOverride,
}

/// A declarative driver-map selection rule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SelectionRule {
    /// Stable non-empty identifier used in decisions and ambiguity errors.
    pub id: String,
    /// Explicit semantic precedence.
    pub precedence: Precedence,
    /// Predicates that must all match. An empty list is a catch-all rule.
    pub matchers: Vec<IdentityMatcher>,
    /// Complete capability map selected by this rule.
    pub drivers: DriverMap,
}

/// Deterministic BLAKE3 digest of a validated, canonical rule set.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuleSetHash([u8; 32]);

impl RuleSetHash {
    /// Returns the raw 32-byte BLAKE3 digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for RuleSetHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(Hash::from_bytes(self.0).to_hex().as_str())
    }
}

impl Serialize for RuleSetHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RuleSetHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Hash::from_hex(&value)
            .map(|hash| Self(*hash.as_bytes()))
            .map_err(serde::de::Error::custom)
    }
}

/// A validated, canonically ordered set of selection rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleSet {
    rules: Vec<SelectionRule>,
    hash: RuleSetHash,
}

impl RuleSet {
    /// Validates and canonicalizes rules before computing their stable hash.
    pub fn new(mut rules: Vec<SelectionRule>) -> Result<Self, RuleSetError> {
        for rule in &mut rules {
            if rule.id.trim().is_empty() {
                return Err(RuleSetError::EmptyRuleId);
            }
            for matcher in &mut rule.matchers {
                if let MatchPattern::OneOf(values) = &mut matcher.pattern {
                    values.sort();
                    if values.is_empty() {
                        return Err(RuleSetError::EmptyOneOf {
                            rule_id: rule.id.clone(),
                            field: matcher.field,
                        });
                    }
                    if values.windows(2).any(|pair| pair[0] == pair[1]) {
                        return Err(RuleSetError::DuplicateOneOfValue {
                            rule_id: rule.id.clone(),
                            field: matcher.field,
                        });
                    }
                }
                if matcher.pattern.first_value().is_some_and(str::is_empty) {
                    return Err(RuleSetError::EmptyPattern {
                        rule_id: rule.id.clone(),
                        field: matcher.field,
                    });
                }
                if matches!(matcher.pattern, MatchPattern::FirmwareVersionRange(_))
                    && !matches!(
                        matcher.field,
                        IdentityField::ManagerFirmware | IdentityField::SystemBiosVersion
                    )
                {
                    return Err(RuleSetError::VersionRangeOnNonFirmwareField {
                        rule_id: rule.id.clone(),
                        field: matcher.field,
                    });
                }
            }
            rule.matchers.sort();
            if rule.matchers.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(RuleSetError::DuplicateMatcher {
                    rule_id: rule.id.clone(),
                });
            }
        }
        rules.sort_by(|left, right| left.id.cmp(&right.id));
        if rules.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(RuleSetError::DuplicateRuleId);
        }
        let hash = hash_rules(&rules);
        Ok(Self { rules, hash })
    }

    /// Returns rules in canonical identifier order.
    pub fn rules(&self) -> &[SelectionRule] {
        &self.rules
    }

    /// Returns the stable digest of this canonical rule set.
    pub const fn hash(&self) -> RuleSetHash {
        self.hash
    }

    /// Selects the unique highest-precedence rule matching `identity`.
    pub fn resolve(
        &self,
        identity: &PlatformIdentity,
    ) -> Result<ResolvedSelection, SelectionError> {
        let matching = self
            .rules
            .iter()
            .filter(|rule| {
                rule.matchers
                    .iter()
                    .all(|matcher| matcher.matches(identity))
            })
            .collect::<Vec<_>>();
        let Some(precedence) = matching.iter().map(|rule| rule.precedence).max() else {
            return Ok(ResolvedSelection {
                rule_id: "standard-default".to_string(),
                drivers: standard_driver_map(),
                rule_set_hash: self.hash,
            });
        };
        let winners = matching
            .into_iter()
            .filter(|rule| rule.precedence == precedence)
            .collect::<Vec<_>>();
        if winners.len() != 1 {
            return Err(SelectionError::Ambiguous {
                precedence,
                rule_ids: winners.iter().map(|rule| rule.id.clone()).collect(),
            });
        }
        let winner = winners[0];
        Ok(ResolvedSelection {
            rule_id: winner.id.clone(),
            drivers: winner.drivers.clone(),
            rule_set_hash: self.hash,
        })
    }
}

/// Failure while validating a selection rule set.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RuleSetError {
    /// A rule identifier is empty or whitespace-only.
    #[error("selection rule id must not be empty")]
    EmptyRuleId,
    /// Two rules use the same identifier.
    #[error("selection rule ids must be unique")]
    DuplicateRuleId,
    /// A matcher contains an empty comparison value.
    #[error("rule {rule_id} has an empty pattern for {field:?}")]
    EmptyPattern {
        /// Rule containing the invalid matcher.
        rule_id: String,
        /// Field examined by the invalid matcher.
        field: IdentityField,
    },
    /// A rule repeats an identical matcher.
    #[error("rule {rule_id} contains a duplicate matcher")]
    DuplicateMatcher {
        /// Rule containing the duplicate.
        rule_id: String,
    },
    /// A one-of matcher has no candidate values.
    #[error("rule {rule_id} has an empty one-of matcher for {field:?}")]
    EmptyOneOf {
        /// Rule containing the invalid matcher.
        rule_id: String,
        /// Field examined by the invalid matcher.
        field: IdentityField,
    },
    /// A one-of matcher repeats a candidate value.
    #[error("rule {rule_id} repeats a one-of value for {field:?}")]
    DuplicateOneOfValue {
        /// Rule containing the invalid matcher.
        rule_id: String,
        /// Field examined by the invalid matcher.
        field: IdentityField,
    },
    /// A firmware range was attached to a non-version identity field.
    #[error("rule {rule_id} applies a firmware range to non-firmware field {field:?}")]
    VersionRangeOnNonFirmwareField {
        /// Rule containing the invalid matcher.
        rule_id: String,
        /// Invalid field.
        field: IdentityField,
    },
}

/// Failure to choose one rule for an identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SelectionError {
    /// Multiple matching rules shared the highest precedence.
    #[error("ambiguous BMC driver selection at precedence {precedence:?}: {rule_ids:?}")]
    Ambiguous {
        /// Shared precedence of the ambiguous rules.
        precedence: Precedence,
        /// Canonically ordered identifiers of the ambiguous rules.
        rule_ids: Vec<String>,
    },
}

/// The selected rule, complete driver map, and source rule-set hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSelection {
    rule_id: String,
    drivers: DriverMap,
    rule_set_hash: RuleSetHash,
}

impl ResolvedSelection {
    /// Returns the selected rule identifier.
    pub fn rule_id(&self) -> &str {
        &self.rule_id
    }

    /// Returns the selected complete driver map.
    pub const fn drivers(&self) -> &DriverMap {
        &self.drivers
    }

    /// Returns the hash of the rule set used for this decision.
    pub const fn rule_set_hash(&self) -> RuleSetHash {
        self.rule_set_hash
    }
}

fn field_values(identity: &PlatformIdentity, field: IdentityField) -> Vec<&str> {
    match field {
        IdentityField::ServiceRootVendor => optional(identity.service_root.vendor.as_deref()),
        IdentityField::ServiceRootProduct => optional(identity.service_root.product.as_deref()),
        IdentityField::ServiceRootOemKey => identity
            .service_root
            .oem_keys
            .iter()
            .map(String::as_str)
            .collect(),
        IdentityField::ManagerModel => optional(
            identity
                .manager
                .as_ref()
                .and_then(|value| value.model.as_deref()),
        ),
        IdentityField::ManagerFirmware => optional(
            identity
                .manager
                .as_ref()
                .and_then(|value| value.firmware.as_deref()),
        ),
        IdentityField::SystemId => identity
            .system
            .as_ref()
            .map(|value| vec![value.id.as_str()])
            .unwrap_or_default(),
        IdentityField::SystemManufacturer => optional(
            identity
                .system
                .as_ref()
                .and_then(|value| value.manufacturer.as_deref()),
        ),
        IdentityField::SystemModel => optional(
            identity
                .system
                .as_ref()
                .and_then(|value| value.model.as_deref()),
        ),
        IdentityField::SystemSku => optional(
            identity
                .system
                .as_ref()
                .and_then(|value| value.sku.as_deref()),
        ),
        IdentityField::SystemPartNumber => optional(
            identity
                .system
                .as_ref()
                .and_then(|value| value.part_number.as_deref()),
        ),
        IdentityField::SystemBiosVersion => optional(
            identity
                .system
                .as_ref()
                .and_then(|value| value.bios_version.as_deref()),
        ),
        IdentityField::ChassisId => identity
            .chassis
            .iter()
            .map(|value| value.id.as_str())
            .collect(),
        IdentityField::ChassisManufacturer => identity
            .chassis
            .iter()
            .filter_map(|value| value.manufacturer.as_deref())
            .collect(),
        IdentityField::ChassisModel => identity
            .chassis
            .iter()
            .filter_map(|value| value.model.as_deref())
            .collect(),
        IdentityField::ChassisPartNumber => identity
            .chassis
            .iter()
            .filter_map(|value| value.part_number.as_deref())
            .collect(),
    }
}

fn optional(value: Option<&str>) -> Vec<&str> {
    value.into_iter().collect()
}

fn standard_driver_map() -> DriverMap {
    DriverMap {
        power: CapabilitySelection::Standard,
        bmc_control: CapabilitySelection::Standard,
        bios: CapabilitySelection::Standard,
        boot_order: CapabilitySelection::Standard,
        secure_boot: CapabilitySelection::Standard,
        lockdown: CapabilitySelection::Standard,
        accounts: CapabilitySelection::Standard,
        firmware: CapabilitySelection::Standard,
        storage: CapabilitySelection::Standard,
        dpu: CapabilitySelection::Standard,
        attestation: CapabilitySelection::Standard,
        console: CapabilitySelection::Standard,
    }
}

fn hash_rules(rules: &[SelectionRule]) -> RuleSetHash {
    let mut hasher = Hasher::new();
    hasher.update(b"bmc-runtime-selection-rules-v2");
    hash_length(&mut hasher, rules.len());
    for rule in rules {
        hash_string(&mut hasher, &rule.id);
        hasher.update(&[rule.precedence as u8]);
        hash_length(&mut hasher, rule.matchers.len());
        for matcher in &rule.matchers {
            hasher.update(&(matcher.field as u8).to_le_bytes());
            matcher.pattern.hash_into(&mut hasher);
        }
        for (_, selection) in rule.drivers.iter() {
            match selection {
                CapabilitySelection::Standard => {
                    hasher.update(&[0]);
                }
                CapabilitySelection::Unsupported => {
                    hasher.update(&[1]);
                }
                CapabilitySelection::Driver(driver) => {
                    hasher.update(&[2]);
                    hash_string(&mut hasher, driver.as_str());
                }
            }
        }
    }
    RuleSetHash(*hasher.finalize().as_bytes())
}

fn hash_string(hasher: &mut Hasher, value: &str) {
    hash_length(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_length(hasher: &mut Hasher, length: usize) {
    hasher.update(&(length as u64).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bmc_platform::{
        Capability, ChassisIdentity, ManagerIdentity, ServiceRootIdentity, SystemIdentity,
    };
    use carbide_test_support::value_scenarios;

    use super::*;

    fn identity() -> PlatformIdentity {
        PlatformIdentity {
            service_root: ServiceRootIdentity {
                vendor: Some("NVIDIA".to_string()),
                product: Some("GB BMC".to_string()),
                oem_keys: vec!["Nvidia".to_string()],
            },
            manager: Some(ManagerIdentity {
                model: Some("OpenBMC".to_string()),
                firmware: Some("1.2.3".to_string()),
            }),
            system: Some(SystemIdentity {
                id: "System_0".to_string(),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("GB300".to_string()),
                sku: Some("DGX".to_string()),
                part_number: Some("900-2G535".to_string()),
                bios_version: Some("2.0".to_string()),
            }),
            chassis: vec![ChassisIdentity {
                id: "GPU_Chassis".to_string(),
                manufacturer: Some("NVIDIA".to_string()),
                model: Some("NVIDIA GB300".to_string()),
                part_number: None,
            }],
        }
    }

    fn drivers(selection: CapabilitySelection) -> DriverMap {
        DriverMap {
            power: selection,
            bmc_control: CapabilitySelection::Standard,
            bios: CapabilitySelection::Standard,
            boot_order: CapabilitySelection::Standard,
            secure_boot: CapabilitySelection::Standard,
            lockdown: CapabilitySelection::Standard,
            accounts: CapabilitySelection::Standard,
            firmware: CapabilitySelection::Standard,
            storage: CapabilitySelection::Unsupported,
            dpu: CapabilitySelection::Unsupported,
            attestation: CapabilitySelection::Standard,
            console: CapabilitySelection::Standard,
        }
    }

    fn rule(id: &str, precedence: Precedence, matcher: IdentityMatcher) -> SelectionRule {
        SelectionRule {
            id: id.to_string(),
            precedence,
            matchers: vec![matcher],
            drivers: drivers(CapabilitySelection::Standard),
        }
    }

    #[test]
    fn match_patterns_cover_every_comparison_mode() {
        value_scenarios!(run = |pattern: MatchPattern| pattern.matches("NVIDIA GB300");
            "matching patterns" {
                MatchPattern::Exact("NVIDIA GB300".to_string()) => true,
                MatchPattern::ExactAsciiCaseInsensitive("nvidia gb300".to_string()) => true,
                MatchPattern::Prefix("NVIDIA".to_string()) => true,
                MatchPattern::Contains("GB300".to_string()) => true,
            }
            "non-matching patterns" {
                MatchPattern::Exact("NVIDIA".to_string()) => false,
                MatchPattern::ExactAsciiCaseInsensitive("nvidia gb200".to_string()) => false,
                MatchPattern::Prefix("GB300".to_string()) => false,
                MatchPattern::Contains("GB200".to_string()) => false,
            }
        );
    }

    #[test]
    fn matchers_read_scalar_and_repeated_identity_fields() {
        let cases = [
            (
                IdentityField::ServiceRootVendor,
                MatchPattern::Exact("NVIDIA".to_string()),
                true,
            ),
            (
                IdentityField::ManagerModel,
                MatchPattern::Exact("OpenBMC".to_string()),
                true,
            ),
            (
                IdentityField::SystemPartNumber,
                MatchPattern::Prefix("900-".to_string()),
                true,
            ),
            (
                IdentityField::ChassisModel,
                MatchPattern::Contains("GB300".to_string()),
                true,
            ),
            (
                IdentityField::ChassisPartNumber,
                MatchPattern::Exact("missing".to_string()),
                false,
            ),
        ];
        let identity = identity();
        for (field, pattern, expected) in cases {
            assert_eq!(
                IdentityMatcher::new(field, pattern).matches(&identity),
                expected
            );
        }
    }

    #[test]
    fn one_of_matches_any_exact_candidate_and_canonicalizes_order() {
        let matcher = IdentityMatcher::new(
            IdentityField::SystemModel,
            MatchPattern::OneOf(vec!["GB200".to_string(), "GB300".to_string()]),
        );
        assert!(matcher.matches(&identity()));

        let left = RuleSet::new(vec![rule(
            "models",
            Precedence::ExactSystemIdentity,
            matcher,
        )])
        .expect("one-of values are valid");
        let right = RuleSet::new(vec![rule(
            "models",
            Precedence::ExactSystemIdentity,
            IdentityMatcher::new(
                IdentityField::SystemModel,
                MatchPattern::OneOf(vec!["GB300".to_string(), "GB200".to_string()]),
            ),
        )])
        .expect("reordered one-of values are valid");
        assert_eq!(left.hash(), right.hash());
    }

    #[test]
    fn firmware_ranges_are_validated_and_inclusive() {
        let range = FirmwareVersionRange::new("1.2.3".to_string(), "2.0".to_string())
            .expect("ordered versions are valid");
        value_scenarios!(run = |version| range.contains(version);
            "inside inclusive range" {
                "1.2.3" => true,
                "1.5" => true,
                "2.0" => true,
            }
            "outside or invalid" {
                "1.2.2" => false,
                "2.0.1" => false,
                "" => false,
            }
        );
        assert_eq!(
            FirmwareVersionRange::new("2.0".to_string(), "1.0".to_string()),
            Err(FirmwareVersionRangeError::Reversed)
        );
    }

    #[test]
    fn extended_matchers_reject_invalid_declarations() {
        let cases = [
            (
                IdentityMatcher::new(IdentityField::SystemModel, MatchPattern::OneOf(Vec::new())),
                RuleSetError::EmptyOneOf {
                    rule_id: "invalid".to_string(),
                    field: IdentityField::SystemModel,
                },
            ),
            (
                IdentityMatcher::new(
                    IdentityField::SystemModel,
                    MatchPattern::OneOf(vec!["GB300".to_string(), "GB300".to_string()]),
                ),
                RuleSetError::DuplicateOneOfValue {
                    rule_id: "invalid".to_string(),
                    field: IdentityField::SystemModel,
                },
            ),
            (
                IdentityMatcher::new(
                    IdentityField::SystemModel,
                    MatchPattern::FirmwareVersionRange(
                        FirmwareVersionRange::new("1.0".to_string(), "2.0".to_string())
                            .expect("fixture range is valid"),
                    ),
                ),
                RuleSetError::VersionRangeOnNonFirmwareField {
                    rule_id: "invalid".to_string(),
                    field: IdentityField::SystemModel,
                },
            ),
        ];

        for (matcher, expected) in cases {
            assert_eq!(
                RuleSet::new(vec![rule(
                    "invalid",
                    Precedence::DeploymentOverride,
                    matcher,
                )]),
                Err(expected)
            );
        }
    }

    #[test]
    fn precedence_order_matches_selection_design() {
        assert!(
            Precedence::StandardDefault < Precedence::VendorManufacturer
                && Precedence::VendorManufacturer < Precedence::BmcProductManager
                && Precedence::BmcProductManager < Precedence::ExactSystemIdentity
                && Precedence::ExactSystemIdentity < Precedence::DeploymentOverride
        );
    }

    #[test]
    fn higher_precedence_rule_wins_independent_of_input_order() {
        let vendor = IdentityMatcher::new(
            IdentityField::ServiceRootVendor,
            MatchPattern::Exact("NVIDIA".to_string()),
        );
        let rules = RuleSet::new(vec![
            rule("specific", Precedence::VendorManufacturer, vendor.clone()),
            rule("fallback", Precedence::StandardDefault, vendor),
        ])
        .expect("rules are valid");

        let resolved = rules.resolve(&identity()).expect("one rule wins");
        assert_eq!(resolved.rule_id(), "specific");
    }

    #[test]
    fn tied_highest_precedence_is_ambiguous_in_canonical_order() {
        let matcher = IdentityMatcher::new(
            IdentityField::SystemModel,
            MatchPattern::Exact("GB300".to_string()),
        );
        let rules = RuleSet::new(vec![
            rule("z-rule", Precedence::ExactSystemIdentity, matcher.clone()),
            rule("a-rule", Precedence::ExactSystemIdentity, matcher),
        ])
        .expect("rules are valid");

        assert_eq!(
            rules.resolve(&identity()),
            Err(SelectionError::Ambiguous {
                precedence: Precedence::ExactSystemIdentity,
                rule_ids: vec!["a-rule".to_string(), "z-rule".to_string()],
            })
        );
    }

    #[test]
    fn rule_hash_is_order_independent_and_semantically_sensitive() {
        let vendor = IdentityMatcher::new(
            IdentityField::ServiceRootVendor,
            MatchPattern::Exact("NVIDIA".to_string()),
        );
        let model = IdentityMatcher::new(
            IdentityField::SystemModel,
            MatchPattern::Exact("GB300".to_string()),
        );
        let first = rule("nvidia", Precedence::VendorManufacturer, vendor);
        let second = rule("gb300", Precedence::ExactSystemIdentity, model);
        let forward = RuleSet::new(vec![first.clone(), second.clone()]).expect("rules are valid");
        let reverse = RuleSet::new(vec![second.clone(), first.clone()]).expect("rules are valid");
        assert_eq!(forward.hash(), reverse.hash());

        let mut changed = second;
        changed.precedence = Precedence::DeploymentOverride;
        let changed =
            RuleSet::new(vec![first, changed]).expect("changed rules remain structurally valid");
        assert_ne!(forward.hash(), changed.hash());
    }

    #[test]
    fn rule_validation_rejects_ambiguous_configuration_artifacts() {
        let matcher = IdentityMatcher::new(
            IdentityField::SystemModel,
            MatchPattern::Exact("GB300".to_string()),
        );
        let cases = [
            (
                vec![rule("", Precedence::StandardDefault, matcher.clone())],
                RuleSetError::EmptyRuleId,
            ),
            (
                vec![
                    rule("duplicate", Precedence::StandardDefault, matcher.clone()),
                    rule("duplicate", Precedence::DeploymentOverride, matcher),
                ],
                RuleSetError::DuplicateRuleId,
            ),
            (
                vec![rule(
                    "empty",
                    Precedence::StandardDefault,
                    IdentityMatcher::new(
                        IdentityField::SystemModel,
                        MatchPattern::Exact(String::new()),
                    ),
                )],
                RuleSetError::EmptyPattern {
                    rule_id: "empty".to_string(),
                    field: IdentityField::SystemModel,
                },
            ),
        ];
        for (rules, expected) in cases {
            assert_eq!(RuleSet::new(rules), Err(expected));
        }
    }

    #[test]
    fn rule_set_hash_serializes_as_hex_and_round_trips() {
        let rules = RuleSet::new(vec![rule(
            "fallback",
            Precedence::StandardDefault,
            IdentityMatcher::new(
                IdentityField::ServiceRootVendor,
                MatchPattern::Exact("NVIDIA".to_string()),
            ),
        )])
        .expect("rules are valid");

        let encoded = serde_json::to_string(&rules.hash()).expect("hash serializes");
        assert_eq!(encoded.len(), 66);
        assert_eq!(
            serde_json::from_str::<RuleSetHash>(&encoded).expect("hash deserializes"),
            rules.hash()
        );
    }

    #[test]
    fn no_matching_rule_uses_complete_standard_default() {
        let rules = RuleSet::new(vec![rule(
            "dell",
            Precedence::VendorManufacturer,
            IdentityMatcher::new(
                IdentityField::ServiceRootVendor,
                MatchPattern::Exact("Dell".to_string()),
            ),
        )])
        .expect("rules are valid");
        let resolved = rules
            .resolve(&identity())
            .expect("built-in default is complete");
        assert_eq!(resolved.rule_id(), "standard-default");
        assert!(
            resolved
                .drivers()
                .iter()
                .all(|(_, driver)| driver == &CapabilitySelection::Standard)
        );
    }

    #[test]
    fn duplicate_matcher_is_rejected_after_canonicalization() {
        let matcher = IdentityMatcher::new(
            IdentityField::SystemModel,
            MatchPattern::Exact("GB300".to_string()),
        );
        let mut duplicate = rule(
            "duplicate-matcher",
            Precedence::StandardDefault,
            matcher.clone(),
        );
        duplicate.matchers.push(matcher);
        assert_eq!(
            RuleSet::new(vec![duplicate]),
            Err(RuleSetError::DuplicateMatcher {
                rule_id: "duplicate-matcher".to_string(),
            })
        );
    }

    #[test]
    fn helper_driver_map_fixture_is_complete() {
        let ids = drivers(CapabilitySelection::Standard)
            .iter()
            .map(|(capability, _)| capability)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), Capability::ALL.len());
    }
}
