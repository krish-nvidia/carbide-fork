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

//! Progress lines, the summary, and redaction of every secret the run knows.

use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use bmc_platform::PlatformIdentity;

use crate::checks::{CheckResult, Ctx, Kind, Outcome, Planned};
use crate::selection::Selection;

const REDACTED: &str = "********";

/// Replaces every known secret in text bound for the terminal.
pub(crate) struct Redactor {
    secrets: Mutex<Vec<String>>,
}

impl Redactor {
    pub(crate) fn new(password: &str) -> Self {
        let redactor = Self {
            secrets: Mutex::new(Vec::new()),
        };
        redactor.add(password);
        redactor
    }

    pub(crate) fn add(&self, secret: &str) {
        if !secret.is_empty() {
            self.secrets
                .lock()
                .expect("secrets lock")
                .push(secret.to_string());
        }
    }

    pub(crate) fn apply(&self, text: &str) -> String {
        self.secrets
            .lock()
            .expect("secrets lock")
            .iter()
            .fold(text.to_string(), |text, secret| {
                text.replace(secret, REDACTED)
            })
    }
}

pub(crate) fn print_identity(identity: &PlatformIdentity) {
    let root = &identity.service_root;
    println!();
    println!(
        "service root: vendor={} product={} oem={}",
        shown(root.vendor.as_deref()),
        shown(root.product.as_deref()),
        root.oem_keys.join(",")
    );
    match &identity.system {
        Some(system) => println!(
            "system {}: manufacturer={} model={} sku={} part={} bios={}",
            system.id,
            shown(system.manufacturer.as_deref()),
            shown(system.model.as_deref()),
            shown(system.sku.as_deref()),
            shown(system.part_number.as_deref()),
            shown(system.bios_version.as_deref()),
        ),
        None => println!("system: none listed"),
    }
    match &identity.manager {
        Some(manager) => println!(
            "manager {}: model={} firmware={}",
            manager.id,
            shown(manager.model.as_deref()),
            shown(manager.firmware.as_deref()),
        ),
        None => println!("manager: none listed"),
    }
    for chassis in &identity.chassis {
        println!(
            "chassis {}: manufacturer={} model={} part={}",
            chassis.id,
            shown(chassis.manufacturer.as_deref()),
            shown(chassis.model.as_deref()),
            shown(chassis.part_number.as_deref()),
        );
    }
    let firmware: Vec<String> = identity
        .firmware_inventory
        .iter()
        .map(|entry| format!("{}={}", entry.id, shown(entry.version.as_deref())))
        .collect();
    println!("firmware: {}", firmware.join(", "));
}

pub(crate) fn print_plan(plan: &[Planned], selection: &Selection) {
    println!();
    println!("{:<6} {:<32} {:<30} PLAN", "KIND", "CHECK", "DRIVER");
    for planned in plan {
        let kind = match planned.check.kind {
            Kind::Read => "read",
            Kind::Apply => "apply",
        };
        let plan = match &planned.skip {
            Some(outcome) => format!("{}: {}", label(outcome), detail(outcome)),
            None => "runs".to_string(),
        };
        println!(
            "{kind:<6} {:<32} {:<30} {plan}",
            planned.check.id,
            selection.driver(planned.check.capability)
        );
    }
}

pub(crate) fn progress(ctx: &Ctx, result: &CheckResult) {
    println!(
        "[{:>11}] {:<32} {:<30} {:>7}  {}",
        label(&result.outcome),
        result.id,
        result.driver,
        seconds(result.elapsed),
        ctx.redactor.apply(detail(&result.outcome))
    );
}

/// Prints the summary and returns the exit code: 0 when nothing failed, 1
/// when a check failed, 3 when a change could not be restored.
pub(crate) fn summary(ctx: &Ctx, results: &[CheckResult]) -> ExitCode {
    let count = |want: fn(&Outcome) -> bool| results.iter().filter(|r| want(&r.outcome)).count();
    let failed = count(|outcome| matches!(outcome, Outcome::Fail(_)));
    println!();
    println!(
        "{} passed, {failed} failed, {} unsupported, {} skipped",
        count(|outcome| matches!(outcome, Outcome::Pass(_))),
        count(|outcome| matches!(outcome, Outcome::Unsupported(_))),
        count(|outcome| matches!(outcome, Outcome::Skipped(_))),
    );
    for (heading, want) in [
        (
            "FAILED",
            (|outcome| matches!(outcome, Outcome::Fail(_))) as fn(&Outcome) -> bool,
        ),
        ("UNSUPPORTED", |outcome| {
            matches!(outcome, Outcome::Unsupported(_))
        }),
        ("SKIPPED", |outcome| matches!(outcome, Outcome::Skipped(_))),
    ] {
        let matching: Vec<&CheckResult> = results.iter().filter(|r| want(&r.outcome)).collect();
        if matching.is_empty() {
            continue;
        }
        println!("{heading}");
        for result in matching {
            println!(
                "  {} ({}, {})\n    {}",
                result.id,
                result.driver,
                seconds(result.elapsed),
                ctx.redactor.apply(detail(&result.outcome))
            );
        }
    }
    let unrestored = ctx.unrestored();
    if !unrestored.is_empty() {
        println!("NOT RESTORED");
        for change in &unrestored {
            println!("  {}", ctx.redactor.apply(change));
        }
        return ExitCode::from(3);
    }
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn label(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Pass(_) => "PASS",
        Outcome::Fail(_) => "FAIL",
        Outcome::Unsupported(_) => "UNSUPPORTED",
        Outcome::Skipped(_) => "SKIPPED",
    }
}

fn detail(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Pass(detail)
        | Outcome::Fail(detail)
        | Outcome::Unsupported(detail)
        | Outcome::Skipped(detail) => detail,
    }
}

fn seconds(elapsed: Duration) -> String {
    format!("{:.1}s", elapsed.as_secs_f64())
}

fn shown(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}
