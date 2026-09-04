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
use bmc_platform::{Capability, CapabilitySelection, DriverMap, PlatformIdentity};
use carbide_utils::has_duplicates;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use version_compare::{Cmp, Version};

/// An identity value available to declarative selection rules.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
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

    fn minimum(&self) -> &str {
        &self.minimum
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
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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
    /// Requires an ASCII case-insensitive substring.
    ContainsAsciiCaseInsensitive(String),
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
            | Self::Contains(value)
            | Self::ContainsAsciiCaseInsensitive(value) => Some(value),
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
            Self::ContainsAsciiCaseInsensitive(value) => candidate
                .to_ascii_lowercase()
                .contains(&value.to_ascii_lowercase()),
            Self::OneOf(values) => values.iter().any(|value| candidate == value),
            Self::FirmwareVersionRange(range) => range.contains(candidate),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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
///
/// Precedence is not declared: it is derived from the most specific identity
/// field the rule reads, so a broad rule can never outrank a narrower one.
/// Rules parsed as deployment overrides rank above every built-in rule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(into = "SelectionRuleWire", try_from = "SelectionRuleWire")]
pub struct SelectionRule {
    /// Stable non-empty identifier used in decisions and ambiguity errors.
    pub id: String,
    /// Capability selected by this rule.
    pub capability: Capability,
    /// Predicates that must all match. An empty list is a catch-all rule.
    pub matchers: Vec<IdentityMatcher>,
    /// Driver selected for the capability.
    pub selection: CapabilitySelection,
    deployment_override: bool,
}

impl SelectionRule {
    /// Creates a built-in rule; its precedence follows from `matchers`.
    pub fn new(
        id: impl Into<String>,
        capability: Capability,
        matchers: Vec<IdentityMatcher>,
        selection: CapabilitySelection,
    ) -> Self {
        Self {
            id: id.into(),
            capability,
            matchers,
            selection,
            deployment_override: false,
        }
    }

    /// The rank this rule competes at.
    pub fn precedence(&self) -> Precedence {
        if self.deployment_override {
            return Precedence::DeploymentOverride;
        }
        derived_precedence(&self.matchers)
    }
}

/// Wire form of a rule: `precedence` is emitted for readers and, when present
/// on input, must agree with the rule's derived rank.
#[derive(Serialize, Deserialize)]
struct SelectionRuleWire {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    precedence: Option<Precedence>,
    capability: Capability,
    #[serde(default)]
    matchers: Vec<IdentityMatcher>,
    selection: CapabilitySelection,
}

impl From<SelectionRule> for SelectionRuleWire {
    fn from(rule: SelectionRule) -> Self {
        Self {
            id: rule.id.clone(),
            precedence: Some(rule.precedence()),
            capability: rule.capability,
            matchers: rule.matchers,
            selection: rule.selection,
        }
    }
}

impl TryFrom<SelectionRuleWire> for SelectionRule {
    type Error = RuleSetError;

    fn try_from(wire: SelectionRuleWire) -> Result<Self, Self::Error> {
        let mut rule = Self::new(wire.id, wire.capability, wire.matchers, wire.selection);
        match wire.precedence {
            None => {}
            Some(Precedence::DeploymentOverride) => rule.deployment_override = true,
            Some(declared) if declared == rule.precedence() => {}
            Some(declared) => {
                let required = rule.precedence();
                return Err(RuleSetError::InvalidPrecedence {
                    rule_id: rule.id,
                    declared,
                    required,
                });
            }
        }
        Ok(rule)
    }
}

/// Deterministic BLAKE3 digest of a validated, canonical rule set.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuleSetHash([u8; 32]);

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
                    if has_duplicates(values.iter()) {
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
            if has_duplicates(&rule.matchers) {
                return Err(RuleSetError::DuplicateMatcher {
                    rule_id: rule.id.clone(),
                });
            }
        }
        rules.sort_by(|left, right| left.id.cmp(&right.id));
        if has_duplicates(rules.iter().map(|rule| &rule.id)) {
            return Err(RuleSetError::DuplicateRuleId);
        }
        let hash = hash_rules(&rules);
        Ok(Self { rules, hash })
    }

    /// Layers deployment overrides parsed from TOML over compiled-in rules.
    ///
    /// The document holds `[[rules]]` tables in the [`SelectionRule`] wire
    /// format. Every parsed rule ranks as a deployment override; a rule that
    /// declares any other precedence is rejected.
    pub fn with_overrides(
        mut built_in: Vec<SelectionRule>,
        overrides: &str,
    ) -> Result<Self, RuleSetError> {
        #[derive(Deserialize)]
        struct Overrides {
            #[serde(default)]
            rules: Vec<SelectionRuleWire>,
        }
        let overrides: Overrides = toml::from_str(overrides)
            .map_err(|error| RuleSetError::InvalidOverrides(error.to_string()))?;
        for wire in overrides.rules {
            if wire
                .precedence
                .is_some_and(|declared| declared != Precedence::DeploymentOverride)
            {
                return Err(RuleSetError::NonDeploymentOverride { rule_id: wire.id });
            }
            let mut rule =
                SelectionRule::new(wire.id, wire.capability, wire.matchers, wire.selection);
            rule.deployment_override = true;
            built_in.push(rule);
        }
        Self::new(built_in)
    }

    /// Returns rules in canonical identifier order.
    pub fn rules(&self) -> &[SelectionRule] {
        &self.rules
    }

    /// Returns the stable digest of this canonical rule set.
    pub const fn hash(&self) -> RuleSetHash {
        self.hash
    }

    /// Resolves every capability independently, taking `defaults` (normally
    /// the driver table's default map) for capabilities no rule matches.
    ///
    /// The highest matching precedence wins; within it, the rule with the most
    /// matchers wins. Two rules tied on both are ambiguous.
    pub fn resolve(
        &self,
        identity: &PlatformIdentity,
        defaults: &DriverMap,
    ) -> Result<ResolvedSelection, SelectionError> {
        let mut drivers = defaults.clone();
        let mut matched_rules = Vec::new();
        for capability in Capability::ALL {
            let matching = self
                .rules
                .iter()
                .filter(|rule| {
                    rule.capability == capability
                        && rule
                            .matchers
                            .iter()
                            .all(|matcher| matcher.matches(identity))
                })
                .collect::<Vec<_>>();
            let Some(precedence) = matching.iter().map(|rule| rule.precedence()).max() else {
                continue;
            };
            // Within one precedence level a rule that had to satisfy more
            // identity evidence is the narrower match.
            let candidates = matching
                .into_iter()
                .filter(|rule| rule.precedence() == precedence)
                .collect::<Vec<_>>();
            let specificity = candidates
                .iter()
                .map(|rule| rule.matchers.len())
                .max()
                .unwrap_or_default();
            let winners = candidates
                .into_iter()
                .filter(|rule| rule.matchers.len() == specificity)
                .collect::<Vec<_>>();
            if winners.len() != 1 {
                return Err(SelectionError::Ambiguous {
                    capability,
                    precedence,
                    rule_ids: winners.iter().map(|rule| rule.id.clone()).collect(),
                });
            }
            let winner = winners[0];
            drivers.set(capability, winner.selection.clone());
            matched_rules.push(MatchedRule {
                capability,
                rule_id: winner.id.clone(),
            });
        }
        Ok(ResolvedSelection {
            drivers,
            matched_rules,
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
    /// The override document is not valid TOML in the rule wire format.
    #[error("deployment override rules are invalid: {0}")]
    InvalidOverrides(String),
    /// An override rule must use deployment-override precedence.
    #[error("deployment override rule {rule_id} does not declare deployment_override precedence")]
    NonDeploymentOverride {
        /// The offending rule.
        rule_id: String,
    },
    /// A rule's declared precedence does not match its identity fields.
    #[error("rule {rule_id} declares {declared:?} precedence but its match requires {required:?}")]
    InvalidPrecedence {
        /// Rule with the mismatched precedence.
        rule_id: String,
        /// Precedence the rule declares.
        declared: Precedence,
        /// Precedence implied by the rule's most specific identity field.
        required: Precedence,
    },
}

/// Failure to choose one rule for an identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SelectionError {
    /// Multiple matching rules shared the highest precedence and matcher count.
    #[error("ambiguous {capability} driver selection at precedence {precedence:?}: {rule_ids:?}")]
    Ambiguous {
        /// Capability with multiple matching rules.
        capability: Capability,
        /// Shared precedence of the ambiguous rules.
        precedence: Precedence,
        /// Canonically ordered identifiers of the ambiguous rules.
        rule_ids: Vec<String>,
    },
}

/// The rule selected for one capability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatchedRule {
    /// Capability selected by this rule.
    pub capability: Capability,
    /// Stable identifier of the selected rule.
    pub rule_id: String,
}

/// The complete driver map, per-capability rule provenance, and source hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSelection {
    /// The selected complete driver map.
    pub drivers: DriverMap,
    /// Selected-rule provenance in stable capability order; capabilities using
    /// their portable default are absent.
    pub matched_rules: Vec<MatchedRule>,
    /// Hash of the rule set used for this decision.
    pub rule_set_hash: RuleSetHash,
}

impl ResolvedSelection {
    /// Returns the selected rule for `capability`, or `None` for the portable default.
    pub fn rule_for(&self, capability: Capability) -> Option<&str> {
        self.matched_rules
            .iter()
            .find(|rule| rule.capability == capability)
            .map(|rule| rule.rule_id.as_str())
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

fn derived_precedence(matchers: &[IdentityMatcher]) -> Precedence {
    matchers
        .iter()
        .filter_map(|matcher| field_precedence(matcher.field))
        .max()
        .unwrap_or(Precedence::StandardDefault)
}

const fn field_precedence(field: IdentityField) -> Option<Precedence> {
    match field {
        IdentityField::ServiceRootVendor
        | IdentityField::ServiceRootOemKey
        | IdentityField::SystemManufacturer
        | IdentityField::ChassisManufacturer => Some(Precedence::VendorManufacturer),
        IdentityField::ServiceRootProduct | IdentityField::ManagerModel => {
            Some(Precedence::BmcProductManager)
        }
        IdentityField::SystemId
        | IdentityField::SystemModel
        | IdentityField::SystemSku
        | IdentityField::SystemPartNumber
        | IdentityField::ChassisId
        | IdentityField::ChassisModel
        | IdentityField::ChassisPartNumber => Some(Precedence::ExactSystemIdentity),
        IdentityField::ManagerFirmware | IdentityField::SystemBiosVersion => None,
    }
}

/// Hashes the canonical (sorted, validated) rules through their wire form, so
/// the digest changes exactly when the serialized rule set does.
fn hash_rules(rules: &[SelectionRule]) -> RuleSetHash {
    let mut hasher = Hasher::new();
    hasher.update(b"bmc-runtime-selection-rules-v4");
    hasher.update(&serde_json::to_vec(rules).expect("validated rules serialize"));
    RuleSetHash(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests;
