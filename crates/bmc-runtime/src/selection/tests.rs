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

fn rule(id: &str, precedence: Precedence, matcher: IdentityMatcher) -> SelectionRule {
    SelectionRule {
        id: id.to_string(),
        precedence,
        capability: Capability::Power,
        matchers: vec![matcher],
        selection: CapabilitySelection::Standard,
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
    assert_eq!(resolved.rule_for(Capability::Power), Some("specific"));
}

#[test]
fn capabilities_resolve_independently() {
    let matcher = IdentityMatcher::new(
        IdentityField::ServiceRootVendor,
        MatchPattern::Exact("NVIDIA".to_string()),
    );
    let mut power = rule(
        "nvidia-power",
        Precedence::VendorManufacturer,
        matcher.clone(),
    );
    power.selection =
        CapabilitySelection::Driver("nvidia-power".parse().expect("fixture driver id is valid"));
    let mut accounts = rule("nvidia-accounts", Precedence::VendorManufacturer, matcher);
    accounts.capability = Capability::Accounts;
    accounts.selection = CapabilitySelection::Unsupported;
    let rules = RuleSet::new(vec![accounts, power]).expect("rules are valid");

    let resolved = rules.resolve(&identity()).expect("capabilities resolve");

    assert_eq!(resolved.rule_for(Capability::Power), Some("nvidia-power"));
    assert_eq!(
        resolved.rule_for(Capability::Accounts),
        Some("nvidia-accounts")
    );
    assert!(matches!(
        resolved.drivers().power,
        CapabilitySelection::Driver(_)
    ));
    assert_eq!(
        resolved.drivers().accounts,
        CapabilitySelection::Unsupported
    );
    assert_eq!(resolved.drivers().bios, CapabilitySelection::Standard);
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
            capability: Capability::Power,
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
fn no_matching_rule_uses_standard_for_every_capability() {
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
        .expect("standard fallback resolves");
    assert!(resolved.matched_rules().is_empty());
    assert!(
        resolved
            .drivers()
            .iter()
            .all(|(_, selection)| selection == &CapabilitySelection::Standard)
    );
}

#[test]
fn explicit_catch_all_rule_overrides_one_capability() {
    let rules = RuleSet::new(vec![SelectionRule {
        id: "standard-default".to_string(),
        precedence: Precedence::StandardDefault,
        capability: Capability::Power,
        matchers: Vec::new(),
        selection: CapabilitySelection::Unsupported,
    }])
    .expect("catch-all rule is valid");

    let resolved = rules.resolve(&identity()).expect("catch-all rule matches");
    assert_eq!(
        resolved.rule_for(Capability::Power),
        Some("standard-default")
    );
    assert_eq!(resolved.drivers().power, CapabilitySelection::Unsupported);
    assert_eq!(resolved.drivers().bios, CapabilitySelection::Standard);
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
    let ids = standard_driver_map()
        .iter()
        .map(|(capability, _)| capability)
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), Capability::ALL.len());
}
