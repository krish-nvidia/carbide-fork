/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use bmc_platform::{
    Capability, ChassisIdentity, EtagMode, FirmwareVersionRange, FirmwareVersionRangeError,
    IdentityField, IdentityMatcher, ManagerIdentity, MatchPattern, Precedence, ServiceRootIdentity,
    SystemIdentity,
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

fn vendor(value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::ServiceRootVendor,
        MatchPattern::Exact(value.to_string()),
    )
}

fn model(value: &str) -> IdentityMatcher {
    IdentityMatcher::new(
        IdentityField::SystemModel,
        MatchPattern::Exact(value.to_string()),
    )
}

/// A rule that pins power to the standard driver.
fn rule(id: &str, matchers: impl IntoIterator<Item = IdentityMatcher>) -> Rule {
    Rule::new(id, matchers).standard([Capability::Power])
}

/// The defaults the compiled catalogue would supply.
fn defaults() -> DriverMap {
    DriverMap::filled(CapabilitySelection::Standard)
        .with(Capability::Lockdown, CapabilitySelection::Unsupported)
        .with(Capability::Storage, CapabilitySelection::Unsupported)
        .with(Capability::Dpu, CapabilitySelection::Unsupported)
        .with(Capability::Console, CapabilitySelection::Unsupported)
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
    let one_of = |values: &[&str]| {
        IdentityMatcher::new(
            IdentityField::SystemModel,
            MatchPattern::OneOf(values.iter().map(|value| (*value).to_string()).collect()),
        )
    };
    assert!(one_of(&["GB200", "GB300"]).matches(&identity()));

    let left = Rules::new(vec![rule("models", [one_of(&["GB200", "GB300"])])])
        .expect("one-of values are valid");
    let right = Rules::new(vec![rule("models", [one_of(&["GB300", "GB200"])])])
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
fn rule_validation_rejects_malformed_declarations() {
    let invalid = |matcher: IdentityMatcher| vec![rule("invalid", [matcher])];
    let cases = [
        (
            invalid(IdentityMatcher::new(
                IdentityField::SystemModel,
                MatchPattern::OneOf(Vec::new()),
            )),
            RuleError::EmptyOneOf {
                id: "invalid".to_string(),
                field: IdentityField::SystemModel,
            },
        ),
        (
            invalid(IdentityMatcher::new(
                IdentityField::SystemModel,
                MatchPattern::OneOf(vec!["GB300".to_string(), "GB300".to_string()]),
            )),
            RuleError::DuplicateOneOfValue {
                id: "invalid".to_string(),
                field: IdentityField::SystemModel,
            },
        ),
        (
            invalid(IdentityMatcher::new(
                IdentityField::SystemModel,
                MatchPattern::FirmwareVersionRange(
                    FirmwareVersionRange::new("1.0".to_string(), "2.0".to_string())
                        .expect("fixture range is valid"),
                ),
            )),
            RuleError::VersionRangeOnNonFirmwareField {
                id: "invalid".to_string(),
                field: IdentityField::SystemModel,
            },
        ),
        (
            invalid(model("")),
            RuleError::EmptyPattern {
                id: "invalid".to_string(),
                field: IdentityField::SystemModel,
            },
        ),
        (
            vec![rule("duplicate-matcher", [model("GB300"), model("GB300")])],
            RuleError::DuplicateMatcher {
                id: "duplicate-matcher".to_string(),
            },
        ),
        (vec![rule("", [model("GB300")])], RuleError::EmptyId),
        (
            vec![
                rule("duplicate", [model("GB300")]),
                rule("duplicate", [model("GB300")]),
            ],
            RuleError::DuplicateId,
        ),
        (
            vec![Rule::new("empty", [model("GB300")])],
            RuleError::EmptyRule {
                id: "empty".to_string(),
            },
        ),
    ];
    for (rules, expected) in cases {
        assert_eq!(Rules::new(rules), Err(expected));
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
    let rules = Rules::new(vec![
        rule("specific", [vendor("NVIDIA")]),
        Rule::new("fallback", []).unsupported([Capability::Power]),
    ])
    .expect("rules are valid");

    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("one rule wins");
    assert_eq!(resolved.rule_for(Capability::Power), Some("specific"));
    assert_eq!(
        resolved.drivers.get(Capability::Power),
        &CapabilitySelection::Standard
    );
}

#[test]
fn capabilities_resolve_independently_across_rules() {
    let rules = Rules::new(vec![
        Rule::new("nvidia", [vendor("NVIDIA")]).drivers([Driver::NvidiaOpenBmcPower]),
        Rule::new("gb300", [model("GB300")]).unsupported([Capability::Accounts]),
    ])
    .expect("rules are valid");

    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("capabilities resolve");

    assert_eq!(resolved.rule_for(Capability::Power), Some("nvidia"));
    assert_eq!(resolved.rule_for(Capability::Accounts), Some("gb300"));
    assert_eq!(resolved.rule_for(Capability::Bios), None);
    assert_eq!(
        resolved.drivers.get(Capability::Power),
        &CapabilitySelection::Driver(Driver::NvidiaOpenBmcPower)
    );
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
    let oem = IdentityMatcher::new(
        IdentityField::ServiceRootOemKey,
        MatchPattern::Exact("Nvidia".to_string()),
    );
    let rules = Rules::new(vec![
        Rule::new("specific", [vendor("NVIDIA"), oem]).unsupported([Capability::Power]),
        rule("broad", [vendor("NVIDIA")]),
    ])
    .expect("rules are valid");

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
fn etag_mode_comes_from_the_narrowest_rule_that_declares_one() {
    let rules = Rules::new(vec![
        rule("broad", [vendor("NVIDIA")]).etag(EtagMode::Wildcard),
        rule("narrow", [model("GB300")]),
    ])
    .expect("rules are valid");
    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("rules resolve");
    assert_eq!(resolved.rule_for(Capability::Power), Some("narrow"));
    assert_eq!(resolved.etag_mode, EtagMode::Wildcard);

    let silent = Rules::new(vec![rule("broad", [vendor("NVIDIA")])]).expect("rules are valid");
    assert_eq!(
        silent
            .resolve(&identity(), &defaults())
            .expect("rules resolve")
            .etag_mode,
        EtagMode::Resource
    );

    let tied = Rules::new(vec![
        rule("a", [model("GB300")]).etag(EtagMode::Wildcard),
        rule("b", [model("GB300")]).etag(EtagMode::Resource),
    ])
    .expect("rules are valid");
    assert!(matches!(
        tied.resolve(&identity(), &defaults()),
        Err(SelectionError::Ambiguous { .. })
    ));
}

#[test]
fn toml_overrides_outrank_built_ins() {
    let built_in = vec![rule("vendor", [vendor("NVIDIA")])];
    let overrides = r#"
        [[rules]]
        id = "site"
        matchers = [{ field = "system_model", pattern = { kind = "contains", value = "GB300" } }]
        selections = { power = "unsupported" }
    "#;

    let resolved = Rules::with_overrides(built_in.clone(), overrides)
        .expect("override document is valid")
        .resolve(&identity(), &defaults())
        .expect("override resolves");
    assert_eq!(resolved.rule_for(Capability::Power), Some("site"));
    assert_eq!(
        resolved.drivers.get(Capability::Power),
        &CapabilitySelection::Unsupported
    );

    assert!(matches!(
        Rules::with_overrides(built_in, "rules = 1"),
        Err(RuleError::InvalidOverrides(_))
    ));
}

#[test]
fn tied_highest_rank_is_ambiguous_in_canonical_order() {
    let rules = Rules::new(vec![
        rule("z-rule", [model("GB300")]),
        rule("a-rule", [model("GB300")]),
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
fn hash_is_order_independent_and_semantically_sensitive() {
    let first = rule("nvidia", [vendor("NVIDIA")]);
    let second = rule("gb300", [model("GB300")]);
    let forward = Rules::new(vec![first.clone(), second.clone()]).expect("rules are valid");
    let reverse = Rules::new(vec![second, first.clone()]).expect("rules are valid");
    assert_eq!(forward.hash(), reverse.hash());

    let changed = Rule::new("gb300", [model("GB300")]).unsupported([Capability::Power]);
    let changed = Rules::new(vec![first, changed]).expect("changed rules remain valid");
    assert_ne!(forward.hash(), changed.hash());

    let encoded = serde_json::to_string(&forward.hash()).expect("hash serializes");
    assert_eq!(encoded.len(), 66);
    assert_eq!(
        serde_json::from_str::<SelectionHash>(&encoded).expect("hash deserializes"),
        forward.hash()
    );
}

#[test]
fn no_matching_rule_uses_each_capabilitys_compiled_default() {
    let rules = Rules::new(vec![rule("dell", [vendor("Dell")])]).expect("rules are valid");
    let resolved = rules
        .resolve(&identity(), &defaults())
        .expect("defaults resolve");
    assert!(resolved.matched_rules.is_empty());
    assert_eq!(resolved.drivers, defaults());
}
