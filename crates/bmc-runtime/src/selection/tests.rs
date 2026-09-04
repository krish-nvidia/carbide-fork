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

use bmc_platform::{
    Capability, ChassisIdentity, DriverMap, ManagerIdentity, ServiceRootIdentity, SystemIdentity,
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
            id: "BMC_0".to_string(),
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

fn rule(id: &str, matcher: IdentityMatcher) -> SelectionRule {
    SelectionRule::new(
        id,
        Capability::Power,
        vec![matcher],
        CapabilitySelection::Standard,
    )
}

/// The defaults the compiled driver table would supply.
fn defaults() -> DriverMap {
    DriverMap::filled(CapabilitySelection::Standard)
        .with(Capability::Lockdown, CapabilitySelection::Unsupported)
        .with(Capability::Storage, CapabilitySelection::Unsupported)
        .with(Capability::Dpu, CapabilitySelection::Unsupported)
        .with(Capability::Console, CapabilitySelection::Unsupported)
}

fn catch_all(id: &str, selection: CapabilitySelection) -> SelectionRule {
    SelectionRule::new(id, Capability::Power, Vec::new(), selection)
}

#[test]
fn match_patterns_cover_every_comparison_mode() {
    value_scenarios!(run = |pattern: MatchPattern| pattern.matches("NVIDIA GB300");
        "matching patterns" {
            MatchPattern::Exact("NVIDIA GB300".to_string()) => true,
            MatchPattern::ExactAsciiCaseInsensitive("nvidia gb300".to_string()) => true,
            MatchPattern::Prefix("NVIDIA".to_string()) => true,
            MatchPattern::Contains("GB300".to_string()) => true,
            MatchPattern::ContainsAsciiCaseInsensitive("nvidia gb".to_string()) => true,
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

    let left = RuleSet::new(vec![rule("models", matcher)]).expect("one-of values are valid");
    let right = RuleSet::new(vec![rule(
        "models",
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
        assert_eq!(RuleSet::new(vec![rule("invalid", matcher,)]), Err(expected));
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
        rule("specific", vendor),
        catch_all("fallback", CapabilitySelection::Standard),
    ])
    .expect("rules are valid");

    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("one rule wins");
    assert_eq!(resolved.rule_for(Capability::Power), Some("specific"));
}

#[test]
fn capabilities_resolve_independently() {
    let matcher = IdentityMatcher::new(
        IdentityField::ServiceRootVendor,
        MatchPattern::Exact("NVIDIA".to_string()),
    );
    let mut power = rule("nvidia-power", matcher.clone());
    power.selection =
        CapabilitySelection::Driver("nvidia-power".parse().expect("fixture driver id is valid"));
    let mut accounts = rule("nvidia-accounts", matcher);
    accounts.capability = Capability::Accounts;
    accounts.selection = CapabilitySelection::Unsupported;
    let rules = RuleSet::new(vec![accounts, power]).expect("rules are valid");

    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("capabilities resolve");

    assert_eq!(resolved.rule_for(Capability::Power), Some("nvidia-power"));
    assert_eq!(
        resolved.rule_for(Capability::Accounts),
        Some("nvidia-accounts")
    );
    assert!(matches!(
        resolved.drivers.get(Capability::Power),
        CapabilitySelection::Driver(_)
    ));
    assert_eq!(
        resolved.drivers.get(Capability::Accounts),
        &CapabilitySelection::Unsupported
    );
    assert_eq!(
        resolved.drivers.get(Capability::Bios),
        &CapabilitySelection::Standard
    );
}

#[test]
fn more_matchers_win_within_one_precedence() {
    let vendor = IdentityMatcher::new(
        IdentityField::ServiceRootVendor,
        MatchPattern::Exact("NVIDIA".to_string()),
    );
    let mut specific = rule("specific", vendor.clone());
    specific.matchers.push(IdentityMatcher::new(
        IdentityField::ServiceRootOemKey,
        MatchPattern::Exact("Nvidia".to_string()),
    ));
    specific.selection = CapabilitySelection::Unsupported;
    let rules = RuleSet::new(vec![specific, rule("broad", vendor)]).expect("rules are valid");

    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("specific rule wins");
    assert_eq!(resolved.rule_for(Capability::Power), Some("specific"));
    assert_eq!(
        resolved.drivers.get(Capability::Power),
        &CapabilitySelection::Unsupported
    );
}

#[test]
fn toml_overrides_outrank_built_ins_and_must_declare_override_precedence() {
    let built_in = vec![rule(
        "vendor",
        IdentityMatcher::new(
            IdentityField::ServiceRootVendor,
            MatchPattern::Exact("NVIDIA".to_string()),
        ),
    )];
    let overrides = r#"
        [[rules]]
        id = "site-power"
        precedence = "deployment_override"
        capability = "power"
        selection = "unsupported"
        matchers = [{ field = "system_model", pattern = { kind = "contains", value = "GB300" } }]
    "#;

    let resolved = RuleSet::with_overrides(built_in.clone(), overrides)
        .expect("override document is valid")
        .resolve(&identity(), &defaults())
        .expect("override resolves");
    assert_eq!(resolved.rule_for(Capability::Power), Some("site-power"));
    assert_eq!(
        resolved.drivers.get(Capability::Power),
        &CapabilitySelection::Unsupported
    );

    let undeclared = overrides.replace("precedence = \"deployment_override\"", "");
    let resolved = RuleSet::with_overrides(built_in.clone(), &undeclared)
        .expect("overrides need not declare precedence")
        .resolve(&identity(), &defaults())
        .expect("override resolves");
    assert_eq!(resolved.rule_for(Capability::Power), Some("site-power"));

    let not_override = overrides.replace("deployment_override", "exact_system_identity");
    assert_eq!(
        RuleSet::with_overrides(built_in.clone(), &not_override),
        Err(RuleSetError::NonDeploymentOverride {
            rule_id: "site-power".to_string(),
        })
    );
    assert!(matches!(
        RuleSet::with_overrides(built_in, "rules = 1"),
        Err(RuleSetError::InvalidOverrides(_))
    ));
}

#[test]
fn tied_highest_precedence_is_ambiguous_in_canonical_order() {
    let matcher = IdentityMatcher::new(
        IdentityField::SystemModel,
        MatchPattern::Exact("GB300".to_string()),
    );
    let rules = RuleSet::new(vec![
        rule("z-rule", matcher.clone()),
        rule("a-rule", matcher),
    ])
    .expect("rules are valid");

    assert_eq!(
        rules.resolve(&identity(), &defaults()),
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
    let first = rule("nvidia", vendor);
    let second = rule("gb300", model);
    let forward = RuleSet::new(vec![first.clone(), second.clone()]).expect("rules are valid");
    let reverse = RuleSet::new(vec![second.clone(), first.clone()]).expect("rules are valid");
    assert_eq!(forward.hash(), reverse.hash());

    let mut changed = second;
    changed.selection = CapabilitySelection::Unsupported;
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
        (vec![rule("", matcher.clone())], RuleSetError::EmptyRuleId),
        (
            vec![
                rule("duplicate", matcher.clone()),
                rule("duplicate", matcher),
            ],
            RuleSetError::DuplicateRuleId,
        ),
        (
            vec![rule(
                "empty",
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
fn no_matching_rule_uses_each_capabilitys_portable_default() {
    let rules = RuleSet::new(vec![rule(
        "dell",
        IdentityMatcher::new(
            IdentityField::ServiceRootVendor,
            MatchPattern::Exact("Dell".to_string()),
        ),
    )])
    .expect("rules are valid");
    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("standard fallback resolves");
    assert!(resolved.matched_rules.is_empty());
    for (capability, selection) in resolved.drivers.iter() {
        let expected = match capability {
            Capability::Lockdown | Capability::Storage | Capability::Dpu | Capability::Console => {
                &CapabilitySelection::Unsupported
            }
            _ => &CapabilitySelection::Standard,
        };
        assert_eq!(selection, expected, "unexpected default for {capability}");
    }
}

#[test]
fn explicit_catch_all_rule_overrides_one_capability() {
    let rules = RuleSet::new(vec![catch_all(
        "standard-default",
        CapabilitySelection::Unsupported,
    )])
    .expect("catch-all rule is valid");

    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("catch-all rule matches");
    assert_eq!(
        resolved.rule_for(Capability::Power),
        Some("standard-default")
    );
    assert_eq!(
        resolved.drivers.get(Capability::Power),
        &CapabilitySelection::Unsupported
    );
    assert_eq!(
        resolved.drivers.get(Capability::Bios),
        &CapabilitySelection::Standard
    );
}

#[test]
fn duplicate_matcher_is_rejected_after_canonicalization() {
    let matcher = IdentityMatcher::new(
        IdentityField::SystemModel,
        MatchPattern::Exact("GB300".to_string()),
    );
    let mut duplicate = rule("duplicate-matcher", matcher.clone());
    duplicate.matchers.push(matcher);
    assert_eq!(
        RuleSet::new(vec![duplicate]),
        Err(RuleSetError::DuplicateMatcher {
            rule_id: "duplicate-matcher".to_string(),
        })
    );
}

#[test]
fn declared_precedence_on_the_wire_must_match_the_derived_rank() {
    let wire = serde_json::json!({
        "id": "wrong-level",
        "precedence": "vendor_manufacturer",
        "capability": "power",
        "matchers": [{"field": "system_model", "pattern": {"kind": "exact", "value": "GB300"}}],
        "selection": "standard"
    });
    assert!(serde_json::from_value::<SelectionRule>(wire.clone()).is_err());
    let mut agreeing = wire;
    agreeing["precedence"] = serde_json::json!("exact_system_identity");
    let rule = serde_json::from_value::<SelectionRule>(agreeing).expect("agreeing rule parses");
    assert_eq!(rule.precedence(), Precedence::ExactSystemIdentity);
    assert_eq!(
        serde_json::to_value(&rule).expect("rule serializes")["precedence"],
        serde_json::json!("exact_system_identity")
    );
}
