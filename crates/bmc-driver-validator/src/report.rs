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

//! Terminal output, and redaction of every secret the run knows.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use bmc_platform::{Capability, PlatformIdentity};
use colored::{ColoredString, Colorize};
use strum::IntoEnumIterator;

use crate::checks::{CheckResult, Ctx, Kind, Outcome, Planned};
use crate::selection::Selection;

const REDACTED: &str = "********";

/// Width output is fitted to when the terminal does not say.
const DEFAULT_WIDTH: usize = 120;

/// Columns of the label in key/value rows.
const KEY_WIDTH: usize = 14;

/// Columns of the check id in check rows.
const CHECK_WIDTH: usize = 32;

/// Columns of the driver id in plan rows.
const DRIVER_WIDTH: usize = 30;

/// Narrowest a wrapped text column gets, however wide the columns before it.
const MIN_TEXT_WIDTH: usize = 30;

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

/// Colors output only on a terminal; `NO_COLOR` and `CLICOLOR_FORCE` still apply.
pub(crate) fn init() {
    if !std::io::stdout().is_terminal() {
        colored::control::set_override(false);
    }
}

pub(crate) fn discovering(address: SocketAddr) {
    println!("{}", format!("Discovering {address} …").dimmed());
}

pub(crate) fn print_identity(address: SocketAddr, identity: &PlatformIdentity) {
    heading(&format!("Platform  {}", address.to_string().dimmed()));
    let root = &identity.service_root;
    let oem = (!root.oem_keys.is_empty()).then(|| format!("oem {}", root.oem_keys.join(", ")));
    row(
        "service root",
        &joined([root.vendor.clone(), root.product.clone(), oem]),
    );
    row(
        "system",
        &identity
            .system
            .as_ref()
            .map_or("none listed".to_string(), |system| {
                joined([
                    Some(system.id.clone()),
                    joined_words([system.manufacturer.as_deref(), system.model.as_deref()]),
                    system.sku.as_ref().map(|sku| format!("sku {sku}")),
                    system
                        .part_number
                        .as_ref()
                        .map(|part| format!("part {part}")),
                    system
                        .bios_version
                        .as_ref()
                        .map(|bios| format!("bios {bios}")),
                ])
            }),
    );
    row(
        "manager",
        &identity
            .manager
            .as_ref()
            .map_or("none listed".to_string(), |manager| {
                joined([
                    Some(manager.id.clone()),
                    manager.model.as_ref().map(|model| format!("model {model}")),
                    manager
                        .firmware
                        .as_ref()
                        .map(|firmware| format!("firmware {firmware}")),
                ])
            }),
    );
    for chassis in &identity.chassis {
        row(
            "chassis",
            &joined([
                Some(chassis.id.clone()),
                joined_words([chassis.manufacturer.as_deref(), chassis.model.as_deref()]),
                chassis
                    .part_number
                    .as_ref()
                    .map(|part| format!("part {part}")),
            ]),
        );
    }
    let firmware: Vec<String> = identity
        .firmware_inventory
        .iter()
        .map(|entry| format!("{}={}", entry.id, entry.version.as_deref().unwrap_or("?")))
        .collect();
    row_wrapped("firmware", &firmware.join("  "));
}

pub(crate) fn print_selection(selection: &Selection) {
    heading("Drivers");
    let driver_width = Capability::iter()
        .map(|capability| selection.driver(capability).len())
        .max()
        .unwrap_or_default();
    for capability in Capability::iter() {
        let driver = format!("{:<driver_width$}", selection.driver(capability));
        let driver = if selection.is_supported(capability) {
            driver.cyan()
        } else {
            driver.dimmed()
        };
        println!(
            "  {:<KEY_WIDTH$} {driver}  {}",
            capability.as_str(),
            selection.source(capability).dimmed()
        );
    }
    println!();
    for (id, matchers) in selection.deciding_rules() {
        row(&format!("rule {id}"), &matchers);
    }
    for capability in selection.overridden() {
        row_wrapped(
            "override",
            &format!(
                "{}: {}",
                capability.as_str(),
                selection.override_note(capability)
            ),
        );
    }
    let quirks = selection.quirks();
    row(
        "quirks",
        &if quirks.is_empty() {
            "none".to_string()
        } else {
            quirks.join(", ")
        },
    );
    row("rules hash", &selection.hash());
}

pub(crate) fn print_plan(plan: &[Planned], selection: &Selection) {
    heading("Plan");
    for planned in plan {
        let kind = match planned.check.kind {
            Kind::Read => "read ",
            Kind::Apply => "apply",
        };
        let (symbol, note) = match &planned.skip {
            Some(outcome) => (symbol(outcome), detail(outcome)),
            None => ("▸".normal(), ""),
        };
        let columns = format!(
            "  {symbol} {:<CHECK_WIDTH$} {} {:<DRIVER_WIDTH$} ",
            planned.check.id,
            kind.dimmed(),
            selection.driver(planned.check.capability)
        );
        print_in_column(
            &columns,
            2 + 2 + CHECK_WIDTH + 1 + 5 + 1 + DRIVER_WIDTH + 1,
            note,
            |line| line.dimmed(),
        );
    }
}

pub(crate) fn checks_heading() {
    heading("Checks");
}

pub(crate) fn progress(ctx: &Ctx, result: &CheckResult) {
    let columns = format!(
        "  {} {:<CHECK_WIDTH$} {:>7}  ",
        symbol(&result.outcome),
        result.id,
        seconds(result.elapsed).dimmed(),
    );
    print_in_column(
        &columns,
        2 + 2 + CHECK_WIDTH + 1 + 7 + 2,
        &ctx.redactor.apply(detail(&result.outcome)),
        |line| tinted(&result.outcome, line),
    );
}

/// Prints the summary and returns the exit code: 0 when nothing failed, 1
/// when a check failed, 3 when a change could not be restored.
pub(crate) fn summary(ctx: &Ctx, results: &[CheckResult], elapsed: Duration) -> ExitCode {
    let count = |want: fn(&Outcome) -> bool| results.iter().filter(|r| want(&r.outcome)).count();
    let failed = count(|outcome| matches!(outcome, Outcome::Fail(_)));
    let counts = [
        format!("{} passed", count(|o| matches!(o, Outcome::Pass(_)))).green(),
        if failed > 0 {
            format!("{failed} failed").red().bold()
        } else {
            "0 failed".normal()
        },
        format!(
            "{} unsupported",
            count(|o| matches!(o, Outcome::Unsupported(_)))
        )
        .yellow(),
        format!("{} skipped", count(|o| matches!(o, Outcome::Skipped(_)))).dimmed(),
    ];
    heading(&format!(
        "Summary  {}  {}",
        counts.map(|count| count.to_string()).join(" · "),
        seconds(elapsed).dimmed()
    ));

    let failures: Vec<&CheckResult> = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Fail(_)))
        .collect();
    if !failures.is_empty() {
        heading(&"Failed".red().bold().to_string());
        for result in failures {
            println!(
                "  {} {}  {}",
                symbol(&result.outcome),
                result.id.bold(),
                result.driver.dimmed()
            );
            for line in wrap(
                &ctx.redactor.apply(detail(&result.outcome)),
                width().saturating_sub(6),
            ) {
                println!("      {line}");
            }
        }
    }
    for (title, want) in [
        (
            "Unsupported",
            (|outcome| matches!(outcome, Outcome::Unsupported(_))) as fn(&Outcome) -> bool,
        ),
        ("Skipped", |outcome| matches!(outcome, Outcome::Skipped(_))),
    ] {
        let matching: Vec<&CheckResult> = results.iter().filter(|r| want(&r.outcome)).collect();
        if matching.is_empty() {
            continue;
        }
        heading(title);
        for result in matching {
            let columns = format!("  {} {:<CHECK_WIDTH$} ", symbol(&result.outcome), result.id);
            print_in_column(
                &columns,
                2 + 2 + CHECK_WIDTH + 1,
                &ctx.redactor.apply(detail(&result.outcome)),
                |line| line.dimmed(),
            );
        }
    }

    let unrestored = ctx.unrestored();
    if !unrestored.is_empty() {
        heading(&"Not restored".red().bold().to_string());
        for change in &unrestored {
            print_in_column(
                &format!("  {} ", "!".red().bold()),
                4,
                &ctx.redactor.apply(change),
                |line| line.normal(),
            );
        }
        return ExitCode::from(3);
    }
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn heading(title: &str) {
    println!();
    println!("{}", title.bold());
}

fn row(key: &str, value: &str) {
    println!("  {} {value}", format!("{key:<KEY_WIDTH$}").dimmed());
}

/// A row whose value wraps under itself.
fn row_wrapped(key: &str, value: &str) {
    let indent = 2 + KEY_WIDTH + 1;
    let mut lines = wrap(value, width().saturating_sub(indent)).into_iter();
    row(key, &lines.next().unwrap_or_default());
    for line in lines {
        println!("{:indent$}{line}", "");
    }
}

fn symbol(outcome: &Outcome) -> ColoredString {
    match outcome {
        Outcome::Pass(_) => "✔".green(),
        Outcome::Fail(_) => "✘".red().bold(),
        Outcome::Unsupported(_) => "○".yellow(),
        Outcome::Skipped(_) => "–".dimmed(),
    }
}

fn tinted(outcome: &Outcome, text: &str) -> ColoredString {
    match outcome {
        Outcome::Pass(_) => text.normal(),
        Outcome::Fail(_) => text.red(),
        Outcome::Unsupported(_) | Outcome::Skipped(_) => text.dimmed(),
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

/// The present parts, separated by ` · `.
fn joined<const N: usize>(parts: [Option<String>; N]) -> String {
    parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

/// The present words, separated by spaces, or `None` when there are none.
fn joined_words<const N: usize>(words: [Option<&str>; N]) -> Option<String> {
    let words: Vec<&str> = words.into_iter().flatten().collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|columns| columns.parse().ok())
        .unwrap_or(DEFAULT_WIDTH)
}

/// Prints `columns`, then `text` wrapped to the space left after its
/// `column_width` characters, with continuation lines aligned under the text.
fn print_in_column(
    columns: &str,
    column_width: usize,
    text: &str,
    style: impl Fn(&str) -> ColoredString,
) {
    let max = width().saturating_sub(column_width).max(MIN_TEXT_WIDTH);
    let mut lines = wrap(text, max).into_iter();
    println!("{columns}{}", style(&lines.next().unwrap_or_default()));
    for line in lines {
        println!("{:column_width$}{}", "", style(&line));
    }
}

/// Breaks `text` at spaces into lines of at most `max` characters, splitting
/// any word longer than a line.
fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let chars: Vec<char> = word.chars().collect();
        for piece in chars.chunks(max) {
            let piece: String = piece.iter().collect();
            if !line.is_empty() && line.chars().count() + 1 + piece.chars().count() > max {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&piece);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
