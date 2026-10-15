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

use std::io::{IsTerminal, Write};
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bmc_platform::{Capability, PlatformIdentity};
use colored::{ColoredString, Colorize};
use strum::IntoEnumIterator;
use tokio::task::JoinHandle;

use crate::checks::{CheckResult, Ctx, Kind, Outcome, Planned};
use crate::ownership::Ownership;
use crate::selection::Selection;

const REDACTED: &str = "********";

/// Width output is fitted to when the terminal does not say.
const DEFAULT_WIDTH: usize = 120;

/// Columns of the label in key/value rows.
const KEY_WIDTH: usize = 14;

/// Columns of the check id in check rows.
const CHECK_WIDTH: usize = 34;

/// Columns before the detail text in check rows.
const DETAIL_COLUMN: usize = 2 + 2 + CHECK_WIDTH + 1 + 7 + 2;

/// Columns of the driver id in plan rows.
const DRIVER_WIDTH: usize = 30;

/// Narrowest a wrapped text column gets, however wide the columns before it.
const MIN_TEXT_WIDTH: usize = 30;

/// Most rows the activity line takes for one step.
const MAX_ACTIVITY_ROWS: usize = 6;

/// Spaces between a field's name and its value.
const FIELD_GAP: usize = 2;

/// Spaces between fields that share a row.
const FIELD_GUTTER: usize = 4;

/// Replaces every known secret in text bound for the terminal.
pub(crate) struct Redactor {
    secrets: Mutex<Vec<String>>,
}

impl Redactor {
    pub(crate) fn new<'a>(secrets: impl IntoIterator<Item = &'a str>) -> Self {
        let redactor = Self {
            secrets: Mutex::new(Vec::new()),
        };
        for secret in secrets {
            redactor.add(secret);
        }
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

/// The line under the results naming the checks running now, with how long
/// they have run and their latest step, redrawn in place. Results are printed
/// through [`Activity::above`] so they never mix with it.
pub(crate) struct Activity {
    state: Arc<Mutex<ActivityState>>,
    ticker: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ActivityState {
    running: Vec<Running>,
    frame: usize,
    /// Rows the last frame drew, which the next clear erases.
    rows: usize,
}

struct Running {
    id: &'static str,
    started: Instant,
    step: String,
}

impl Activity {
    /// Draws nothing unless `enabled`.
    pub(crate) fn new(enabled: bool) -> Self {
        const FRAME_INTERVAL: Duration = Duration::from_millis(100);
        let state = Arc::new(Mutex::new(ActivityState::default()));
        let ticker = enabled.then(|| {
            let state = state.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(FRAME_INTERVAL);
                loop {
                    interval.tick().await;
                    state.lock().expect("activity lock").draw();
                }
            })
        });
        Self { state, ticker }
    }

    pub(crate) fn start(&self, id: &'static str, started: Instant) {
        self.state
            .lock()
            .expect("activity lock")
            .running
            .push(Running {
                id,
                started,
                step: String::new(),
            });
    }

    /// Records `step` as what check `id` is doing now.
    pub(crate) fn step(&self, id: &str, step: String) {
        if let Some(running) = self
            .state
            .lock()
            .expect("activity lock")
            .running
            .iter_mut()
            .find(|running| running.id == id)
        {
            running.step = step;
        }
    }

    pub(crate) fn finish(&self, id: &str) {
        self.state
            .lock()
            .expect("activity lock")
            .running
            .retain(|running| running.id != id);
    }

    /// Runs `print` with the line cleared; the next frame redraws it below.
    pub(crate) fn above(&self, print: impl FnOnce()) {
        let mut state = self.state.lock().expect("activity lock");
        state.clear();
        print();
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        if let Some(ticker) = self.ticker.take() {
            ticker.abort();
        }
        self.state.lock().expect("activity lock").clear();
    }
}

impl ActivityState {
    fn draw(&mut self) {
        const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let Some(first) = self.running.first() else {
            self.clear();
            return;
        };
        self.frame = (self.frame + 1) % FRAMES.len();
        let mut lines = self.step_lines(width()).into_iter();
        let mut frame = format!(
            "  {} {:<CHECK_WIDTH$} {:>7}  {}",
            FRAMES[self.frame].to_string().cyan(),
            first.id,
            seconds(first.started.elapsed()).dimmed(),
            lines.next().unwrap_or_default().dimmed()
        );
        let mut rows = 1;
        for line in lines {
            frame.push_str(&format!("\n{:DETAIL_COLUMN$}{}", "", line.dimmed()));
            rows += 1;
        }
        self.clear();
        // Autowrap is off while drawing: a row the terminal wrapped itself
        // would not be counted, and would survive the next clear.
        print!("\x1b[?7l{frame}\x1b[?7h");
        let _ = std::io::stdout().flush();
        self.rows = rows;
    }

    /// What the running checks are doing, wrapped to the detail column of a
    /// `width`-column terminal. A message carrying a whole error page would
    /// fill the screen, so it is cut at [`MAX_ACTIVITY_ROWS`]; the check's
    /// result prints it in full.
    fn step_lines(&self, width: usize) -> Vec<String> {
        let step = match self.running.as_slice() {
            [] => return Vec::new(),
            [only] => only.step.clone(),
            [_, others @ ..] => format!("and {} more running", others.len()),
        };
        let mut lines = wrap(
            &step,
            width.saturating_sub(DETAIL_COLUMN).max(MIN_TEXT_WIDTH),
        );
        if lines.len() > MAX_ACTIVITY_ROWS {
            lines.truncate(MAX_ACTIVITY_ROWS);
            if let Some(last) = lines.last_mut() {
                last.push_str(" …");
            }
        }
        lines
    }

    /// Erases the rows the last frame drew, leaving the cursor where it began.
    fn clear(&mut self) {
        if self.rows == 0 {
            return;
        }
        print!("\r\x1b[2K{}", "\x1b[1A\x1b[2K".repeat(self.rows - 1));
        let _ = std::io::stdout().flush();
        self.rows = 0;
    }
}

/// Colors output only on a terminal; `NO_COLOR` and `CLICOLOR_FORCE` still apply.
pub(crate) fn init() {
    if !std::io::stdout().is_terminal() {
        colored::control::set_override(false);
    }
}

pub(crate) fn taking_ownership(address: SocketAddr) {
    println!(
        "{}",
        format!("Replacing the factory password of {address} …").dimmed()
    );
}

pub(crate) fn print_ownership(ownership: &Ownership) {
    heading("Ownership");
    row("password", &ownership.summary);
    if let Some(driver) = &ownership.accounts_driver {
        row(
            "accounts",
            &format!("{driver}, selected from the ServiceRoot alone"),
        );
    }
}

pub(crate) fn discovering(address: SocketAddr) {
    println!("{}", format!("Discovering {address} …").dimmed());
}

pub(crate) fn warn(message: &str) {
    println!();
    print_in_column(&format!("  {} ", "!".yellow().bold()), 4, message, |line| {
        line.yellow()
    });
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
    ctx.activity.above(|| {
        let columns = format!(
            "  {} {:<CHECK_WIDTH$} {:>7}  ",
            symbol(&result.outcome),
            result.id,
            seconds(result.elapsed).dimmed(),
        );
        print_in_column(
            &columns,
            DETAIL_COLUMN,
            &ctx.redactor.apply(detail(&result.outcome)),
            |line| tinted(&result.outcome, line),
        );
        if let Outcome::Pass(detail) = &result.outcome {
            print_fields(ctx, DETAIL_COLUMN, &detail.fields);
        }
    });
}

/// Prints name/value pairs as an aligned grid starting at column `indent`,
/// as many pairs per row as fit.
fn print_fields(ctx: &Ctx, indent: usize, fields: &[(String, String)]) {
    if fields.is_empty() {
        return;
    }
    let fields: Vec<(String, String)> = fields
        .iter()
        .map(|(name, value)| (ctx.redactor.apply(name), ctx.redactor.apply(value)))
        .collect();
    let name_width = fields
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or_default();
    let value_width = fields
        .iter()
        .map(|(_, value)| value.chars().count())
        .max()
        .unwrap_or_default();
    let available = width().saturating_sub(indent).max(MIN_TEXT_WIDTH);
    let cell = name_width + FIELD_GAP + value_width;
    let per_row = ((available + FIELD_GUTTER) / (cell + FIELD_GUTTER)).max(1);
    if per_row == 1 {
        for (name, value) in &fields {
            let columns = format!(
                "{:indent$}{}{:FIELD_GAP$}",
                "",
                format!("{name:<name_width$}").dimmed(),
                ""
            );
            print_in_column(&columns, indent + name_width + FIELD_GAP, value, |line| {
                line.normal()
            });
        }
        return;
    }
    for row in fields.chunks(per_row) {
        let cells: Vec<String> = row
            .iter()
            .map(|(name, value)| {
                format!(
                    "{}{:FIELD_GAP$}{value:<value_width$}",
                    format!("{name:<name_width$}").dimmed(),
                    ""
                )
            })
            .collect();
        println!(
            "{:indent$}{}",
            "",
            cells.join(&" ".repeat(FIELD_GUTTER)).trim_end()
        );
    }
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
        format!("{} blocked", count(|o| matches!(o, Outcome::Blocked(_)))).dimmed(),
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
        ("Blocked", |outcome| matches!(outcome, Outcome::Blocked(_))),
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
        Outcome::Fail(_) | Outcome::Unreachable(_) => "✘".red().bold(),
        Outcome::Unsupported(_) => "○".yellow(),
        Outcome::Skipped(_) | Outcome::Blocked(_) => "–".dimmed(),
    }
}

fn tinted(outcome: &Outcome, text: &str) -> ColoredString {
    match outcome {
        Outcome::Pass(_) => text.normal(),
        Outcome::Fail(_) | Outcome::Unreachable(_) => text.red(),
        Outcome::Unsupported(_) | Outcome::Skipped(_) | Outcome::Blocked(_) => text.dimmed(),
    }
}

fn detail(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Pass(detail) => &detail.summary,
        Outcome::Fail(detail)
        | Outcome::Unreachable(detail)
        | Outcome::Unsupported(detail)
        | Outcome::Skipped(detail)
        | Outcome::Blocked(detail) => detail,
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

/// The terminal's width, read afresh so a resize applies; `COLUMNS` or
/// [`DEFAULT_WIDTH`] when stdout is not a terminal.
fn width() -> usize {
    terminal_width()
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|columns| columns.parse().ok())
        })
        .unwrap_or(DEFAULT_WIDTH)
}

fn terminal_width() -> Option<usize> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ only writes a `winsize` into the pointer it is
    // given, which points at a live local for the whole call.
    let status = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    (status == 0 && size.ws_col > 0).then_some(usize::from(size.ws_col))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn running(steps: &[&str]) -> ActivityState {
        ActivityState {
            running: steps
                .iter()
                .map(|step| Running {
                    id: "check",
                    started: Instant::now(),
                    step: step.to_string(),
                })
                .collect(),
            ..ActivityState::default()
        }
    }

    /// Steps wrap within the detail column instead of being cut off, up to a
    /// cap that keeps an error page from filling the screen.
    #[test]
    fn steps_wrap_within_the_detail_column() {
        let width = DETAIL_COLUMN + 40;
        let words = |count: usize| vec!["word"; count].join(" ");
        let cases = [
            (
                "fits on one row",
                running(&["restarting the host"]),
                1,
                false,
            ),
            ("wraps onto more rows", running(&[&words(20)]), 3, false),
            (
                "cut at the cap",
                running(&[&words(200)]),
                MAX_ACTIVITY_ROWS,
                true,
            ),
            ("several running", running(&["a", "b", "c"]), 1, false),
        ];
        for (scenario, state, rows, cut) in cases {
            let lines = state.step_lines(width);
            assert_eq!(lines.len(), rows, "{scenario}: {lines:?}");
            assert!(
                lines
                    .iter()
                    .all(|line| line.chars().count() <= 40 + " …".len()),
                "{scenario}: a row is wider than the column: {lines:?}"
            );
            assert_eq!(
                lines.last().is_some_and(|line| line.ends_with(" …")),
                cut,
                "{scenario}"
            );
        }
        assert_eq!(
            running(&["a", "b", "c"]).step_lines(width),
            ["and 2 more running"]
        );
    }
}
