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

//! Driver selection through the compiled rules, with `--driver` overrides
//! layered on as a deployment-override rule.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::SocketAddr;

use bmc_drivers::{
    CapabilitySelection, Driver, Drivers, ResolvedSelection, Rule, Rules, built_in_rules,
    rules_with_overrides,
};
use bmc_platform::{Capability, PlatformIdentity};
use bmc_runtime::BmcRef;
use strum::IntoEnumIterator;

use crate::connect::credential_key;

/// The id of the rule `--driver` overrides become.
const OVERRIDE_RULE: &str = "bmc-validate-override";

pub(crate) struct Selection {
    resolved: ResolvedSelection,
    /// What the compiled rules alone select, to explain overrides against.
    built_in: ResolvedSelection,
    built_in_rules: Rules,
    overrides: BTreeMap<Capability, CapabilitySelection>,
}

impl Selection {
    /// Resolves `identity` through the compiled rules plus `overrides`,
    /// rejecting overrides the compiled catalogue cannot serve.
    pub(crate) fn resolve(
        identity: &PlatformIdentity,
        overrides: &[String],
    ) -> Result<Self, String> {
        let defaults = Drivers::default_map();
        let built_in_rules = built_in_rules();
        let built_in = built_in_rules
            .resolve(identity, &defaults)
            .map_err(|error| format!("selection failed: {error}"))?;

        let mut pins = BTreeMap::new();
        for value in overrides {
            let (capability, selection) = parse_override(value)?;
            if pins.insert(capability, selection).is_some() {
                return Err(format!("--driver pins {capability} more than once"));
            }
        }
        let rules = if pins.is_empty() {
            built_in_rules.clone()
        } else {
            rules_with_overrides(&override_document(&pins))
                .map_err(|error| format!("invalid --driver override: {error}"))?
        };
        Drivers::validate_rules(&rules)
            .map_err(|error| format!("incompatible --driver override: {error}"))?;
        let resolved = rules
            .resolve(identity, &defaults)
            .map_err(|error| format!("selection failed: {error}"))?;
        Drivers::validate_map(&resolved.drivers)
            .map_err(|error| format!("selected driver map is not servable: {error}"))?;

        Ok(Self {
            resolved,
            built_in,
            built_in_rules,
            overrides: pins,
        })
    }

    /// The endpoint `bmc-runtime` connects with, carrying this selection.
    pub(crate) fn endpoint(
        &self,
        address: SocketAddr,
        identity: PlatformIdentity,
    ) -> Result<BmcRef, String> {
        BmcRef::new(address, credential_key(), identity, self.resolved.clone())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn is_supported(&self, capability: Capability) -> bool {
        self.resolved.drivers.get(capability) != &CapabilitySelection::Unsupported
    }

    /// The selected driver's id, `standard`, or `unsupported`.
    pub(crate) fn driver(&self, capability: Capability) -> String {
        label(self.resolved.drivers.get(capability))
    }

    /// Why `capability` has its selection.
    pub(crate) fn reason(&self, capability: Capability) -> String {
        let selection = self.resolved.drivers.get(capability);
        if self.overrides.contains_key(&capability) {
            let built_in = self.built_in.drivers.get(capability);
            let rules_choice = format!(
                "{} ({})",
                label(built_in),
                rule_reason(&self.built_in, &self.built_in_rules, capability)
            );
            if built_in == selection {
                return format!("--driver override, same as the rules ({rules_choice})");
            }
            let mut reason = format!("--driver override; the rules select {rules_choice}");
            if let CapabilitySelection::Driver(_) = selection {
                let selecting: Vec<&str> = self
                    .built_in_rules
                    .rules()
                    .iter()
                    .filter(|rule| rule.selections.get(&capability) == Some(selection))
                    .map(|rule| rule.id.as_str())
                    .collect();
                if selecting.is_empty() {
                    reason.push_str("; no compiled rule selects this driver");
                } else {
                    let _ = write!(
                        reason,
                        "; compiled for rule {}, which does not match this BMC",
                        selecting.join(", ")
                    );
                }
            }
            return reason;
        }
        rule_reason(&self.resolved, &self.built_in_rules, capability)
    }

    pub(crate) fn print(&self) {
        println!();
        println!("{:<13} {:<30} SELECTED BECAUSE", "CAPABILITY", "DRIVER");
        for capability in Capability::iter() {
            println!(
                "{:<13} {:<30} {}",
                capability.as_str(),
                self.driver(capability),
                self.reason(capability)
            );
        }
        let quirks: Vec<String> = self
            .resolved
            .quirks
            .iter()
            .map(|quirk| format!("{quirk:?}"))
            .collect();
        println!(
            "quirks: {}",
            if quirks.is_empty() {
                "none".to_string()
            } else {
                quirks.join(", ")
            }
        );
        println!("rules hash: {}", self.resolved.hash);
    }
}

/// Parses `<capability>=<selection>` or a compiled driver id alone.
fn parse_override(value: &str) -> Result<(Capability, CapabilitySelection), String> {
    let Some((capability, selection)) = value.split_once('=') else {
        let driver: Driver = value
            .parse()
            .map_err(|error| format!("--driver {value}: {error}"))?;
        let capability = driver.capability().ok_or_else(|| {
            format!(
                "--driver {value}: no compiled driver has this id; known ids: {}",
                known_ids()
            )
        })?;
        return Ok((capability, CapabilitySelection::Driver(driver)));
    };
    let capability: Capability = capability
        .parse()
        .map_err(|_| format!("--driver {value}: unknown capability {capability}"))?;
    let selection = match selection {
        "standard" => CapabilitySelection::Standard,
        "unsupported" => CapabilitySelection::Unsupported,
        id => CapabilitySelection::Driver(
            id.parse()
                .map_err(|error| format!("--driver {value}: {error}"))?,
        ),
    };
    Ok((capability, selection))
}

fn known_ids() -> String {
    Driver::ALL
        .iter()
        .map(Driver::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A deployment-override document with one catch-all rule holding `pins`.
fn override_document(pins: &BTreeMap<Capability, CapabilitySelection>) -> String {
    let mut document = format!("[[rules]]\nid = \"{OVERRIDE_RULE}\"\n[rules.selections]\n");
    for (capability, selection) in pins {
        let _ = writeln!(
            document,
            "{} = \"{}\"",
            capability.as_str(),
            label(selection)
        );
    }
    document
}

fn label(selection: &CapabilitySelection) -> String {
    match selection {
        CapabilitySelection::Standard => "standard".to_string(),
        CapabilitySelection::Unsupported => "unsupported".to_string(),
        CapabilitySelection::Driver(driver) => driver.as_str().to_string(),
    }
}

fn rule_reason(resolved: &ResolvedSelection, rules: &Rules, capability: Capability) -> String {
    match resolved.rule_for(capability) {
        Some(id) => match rules.rules().iter().find(|rule| rule.id == id) {
            Some(rule) => format!("rule {id}: {}", matchers(rule)),
            None => format!("rule {id}"),
        },
        None if resolved.drivers.get(capability) == &CapabilitySelection::Standard => {
            "no rule sets it; compiled standard driver".to_string()
        }
        None => "no rule sets it and no standard driver is compiled".to_string(),
    }
}

fn matchers(rule: &Rule) -> String {
    if rule.matchers.is_empty() {
        return "matches every BMC".to_string();
    }
    rule.matchers
        .iter()
        .map(|matcher| format!("{:?} {:?}", matcher.field, matcher.pattern))
        .collect::<Vec<_>>()
        .join(", ")
}
