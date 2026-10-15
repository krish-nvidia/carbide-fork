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

//! The check registry, planning, and the runner.
//!
//! A check is an id, the capability whose selected driver it exercises, and
//! an async function that calls that driver through the connected BMC the
//! way a controller does. [`reads`] and [`apply`] hold the checks a default
//! or `--apply` run picks from; [`validate`] arranges them with its own
//! steps into the `--validate` sequence. The runner keeps plan order:
//! consecutive read checks run concurrently, every other check runs alone,
//! and a check is skipped when one it follows did not pass.

/// Registers an async check function returning text or a [`Detail`].
macro_rules! run {
    ($check:path) => {
        |ctx| Box::pin(async move { $check(ctx).await.map(Into::into) })
    };
}

mod apply;
mod observe;
mod ops;
mod reads;
mod validate;

use std::fmt::Debug;
use std::io::IsTerminal;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bmc_drivers::CatalogError;
use bmc_platform::{
    BiosSettings, BootInterfaceSelector, BootOrderStatus, Capability, PlatformError,
};
use bmc_runtime::ConnectedBmc;
use carbide_redfish::nv_redfish::RedfishBmc;
use colored::Colorize;
use futures::StreamExt;
use mac_address::MacAddress;
use serde_json::Value;
pub(crate) use validate::EndState;

use crate::connect::Target;
use crate::report::{self, Activity, Redactor};
use crate::selection::Selection;

/// What the operator supplied for checks to use.
pub(crate) struct Inputs {
    /// Whether IPMI is attached to the connected BMC.
    pub(crate) ipmi: bool,
    /// MAC address of the interface boot order puts first.
    pub(crate) boot_mac: Option<MacAddress>,
    pub(crate) bios_profile: BiosSettings,
    pub(crate) ntp_servers: Vec<String>,
    pub(crate) uefi_password: Option<UefiPassword>,
    pub(crate) allow_ac_cycle: bool,
    /// PEM of the platform key secure boot enrolls.
    pub(crate) platform_key: Option<String>,
    /// The host BMC that power-cycles a DPU's host.
    pub(crate) host_bmc: Option<Target>,
    /// The factory account, which a factory reset must restore.
    pub(crate) factory: Option<Target>,
    /// Whether apply checks trace each step to stderr.
    pub(crate) verbose: bool,
}

pub(crate) struct UefiPassword {
    /// The password the firmware has now; empty when none is set.
    pub(crate) current: String,
    pub(crate) new: String,
}

/// An input a check cannot run without.
#[derive(Clone, Copy, Debug)]
enum Needs {
    BootInterface,
    NtpServers,
    UefiPassword,
    AcCycle,
    HostBmc,
}

impl Needs {
    /// How a check ends without the input: skipped, naming its flag.
    fn skipped(self) -> Outcome {
        let flag = match self {
            Self::BootInterface => "--boot-mac",
            Self::NtpServers => "--ntp-server",
            Self::UefiPassword => "--uefi-password",
            Self::AcCycle => "--allow-ac-cycle",
            Self::HostBmc => "--host-bmc",
        };
        Outcome::Skipped(format!("needs {flag}"))
    }
}

/// `value`, or the skip for the input that supplies it.
fn needed<T>(value: Option<T>, needs: Needs) -> Result<T, Outcome> {
    value.ok_or_else(|| needs.skipped())
}

impl Inputs {
    fn boot_interface(&self) -> Option<BootInterfaceSelector> {
        self.boot_mac.map(BootInterfaceSelector::Mac)
    }

    fn has(&self, needs: Needs) -> bool {
        match needs {
            Needs::BootInterface => self.boot_mac.is_some(),
            Needs::NtpServers => !self.ntp_servers.is_empty(),
            Needs::UefiPassword => self.uefi_password.is_some(),
            Needs::AcCycle => self.allow_ac_cycle,
            Needs::HostBmc => self.host_bmc.is_some(),
        }
    }
}

/// State shared by every check of one run.
pub(crate) struct Ctx {
    pub(crate) bmc: ConnectedBmc<RedfishBmc>,
    selection: Selection,
    target: Target,
    inputs: Inputs,
    pub(crate) redactor: Arc<Redactor>,
    /// The live line of running checks, without `--verbose` on a terminal.
    pub(crate) activity: Activity,
    /// Changes a check made and could not undo.
    not_restored: Mutex<Vec<String>>,
    /// The apply check running now and when it started, for trace lines.
    tracing: Mutex<Option<(&'static str, Instant)>>,
}

impl Ctx {
    pub(crate) fn new(
        bmc: ConnectedBmc<RedfishBmc>,
        selection: Selection,
        target: Target,
        inputs: Inputs,
        redactor: Arc<Redactor>,
    ) -> Self {
        let activity = Activity::new(!inputs.verbose && std::io::stdout().is_terminal());
        Self {
            bmc,
            selection,
            target,
            inputs,
            redactor,
            activity,
            not_restored: Mutex::new(Vec::new()),
            tracing: Mutex::new(None),
        }
    }

    /// Reports one step of the running apply check.
    fn trace(&self, step: impl AsRef<str>) {
        let tracing = *self.tracing.lock().expect("tracing lock");
        if let Some((check, started)) = tracing {
            self.trace_as(check, started, step);
        }
    }

    /// Reports one step of `check`, which started at `started`: printed to
    /// stderr with `--verbose`, shown on the activity line otherwise.
    fn trace_as(&self, check: &str, started: Instant, step: impl AsRef<str>) {
        let step = self.redactor.apply(step.as_ref());
        if !self.inputs.verbose {
            self.activity.step(check, step);
            return;
        }
        eprintln!(
            "    {} {step}",
            format!("{check} +{:.1}s", started.elapsed().as_secs_f64()).dimmed(),
        );
    }

    fn not_restored(&self, change: String) {
        self.not_restored
            .lock()
            .expect("not-restored lock")
            .push(change);
    }

    pub(crate) fn unrestored(&self) -> Vec<String> {
        self.not_restored.lock().expect("not-restored lock").clone()
    }
}

/// How a check ended.
#[derive(Debug)]
pub(crate) enum Outcome {
    Pass(Detail),
    Fail(String),
    /// The BMC did not answer or reported itself temporarily unavailable, as
    /// while a reset, a settings change, or a host POST occupies it. The
    /// runner runs the check again until it answers or the wait runs out.
    Unreachable(String),
    /// The selection or the driver reports the operation unsupported.
    Unsupported(String),
    /// The check could not run: an input is missing or the platform lacks
    /// what it needs.
    Skipped(String),
    /// A check this one follows failed or was blocked itself.
    Blocked(String),
}

/// What a passing check observed: a one-line summary and the name/value
/// fields shown beneath it.
#[derive(Debug, Default)]
pub(crate) struct Detail {
    pub(crate) summary: String,
    pub(crate) fields: Vec<(String, String)>,
}

impl From<String> for Detail {
    fn from(summary: String) -> Self {
        Self {
            summary,
            fields: Vec::new(),
        }
    }
}

/// A check's text result, or how else it ended.
pub(crate) type Step = Result<String, Outcome>;

/// A check's result with any fields it observed.
pub(crate) type Checked = Result<Detail, Outcome>;

type CheckFn = for<'a> fn(&'a Ctx) -> Pin<Box<dyn Future<Output = Checked> + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// Only reads the BMC.
    Read,
    /// Changes the BMC or host.
    Apply,
}

pub(crate) struct Check {
    pub(crate) id: &'static str,
    pub(crate) capability: Capability,
    pub(crate) kind: Kind,
    needs: Option<Needs>,
    /// Checks that must not have failed for this one to run.
    after: &'static [&'static str],
    /// Whether the check may run again when it finds the BMC unreachable;
    /// not for one whose repeat would redo a change it may already have made.
    rerun: bool,
    timeout: Duration,
    run: CheckFn,
}

impl Check {
    fn read(id: &'static str, capability: Capability, run: CheckFn) -> Self {
        Self {
            id,
            capability,
            kind: Kind::Read,
            needs: None,
            after: &[],
            rerun: true,
            timeout: Duration::from_secs(60),
            run,
        }
    }

    /// Apply checks bound their own waits so they can restore what they
    /// changed; the timeout only backstops a hung request.
    fn apply(id: &'static str, capability: Capability, run: CheckFn) -> Self {
        Self {
            kind: Kind::Apply,
            timeout: Duration::from_secs(2 * 60 * 60),
            ..Self::read(id, capability, run)
        }
    }

    fn timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    fn needs(self, needs: Needs) -> Self {
        Self {
            needs: Some(needs),
            ..self
        }
    }

    fn after(self, after: &'static [&'static str]) -> Self {
        Self { after, ..self }
    }

    /// Fails the check, rather than running it again, when it finds the BMC
    /// unreachable.
    fn once(self) -> Self {
        Self {
            rerun: false,
            ..self
        }
    }
}

pub(crate) struct Planned {
    pub(crate) check: Check,
    /// Why the check will not run, decided before any check runs.
    pub(crate) skip: Option<Outcome>,
}

pub(crate) struct CheckResult {
    pub(crate) id: &'static str,
    pub(crate) driver: String,
    pub(crate) outcome: Outcome,
    pub(crate) elapsed: Duration,
}

/// Which checks a run plans.
pub(crate) enum Scope<'a> {
    /// Every read check, plus the apply checks with `apply`, narrowed to the
    /// capabilities in `only` and the checks in `ids`.
    Checks {
        apply: bool,
        only: &'a [Capability],
        ids: &'a [String],
    },
    /// The `--validate` sequence.
    Validate(EndState),
}

pub(crate) fn plan(
    selection: &Selection,
    inputs: &Inputs,
    scope: Scope<'_>,
) -> Result<Vec<Planned>, String> {
    let checks = match scope {
        Scope::Checks { apply, only, ids } => chosen(apply, only, ids)?,
        Scope::Validate(end_state) => {
            validate::checks(selection.is_supported(Capability::Dpu), end_state)
        }
    };
    Ok(checks
        .into_iter()
        .map(|check| {
            let skip = if !selection.is_supported(check.capability) {
                Some(Outcome::Unsupported(selection.reason(check.capability)))
            } else {
                check
                    .needs
                    .filter(|needs| !inputs.has(*needs))
                    .map(Needs::skipped)
            };
            Planned { check, skip }
        })
        .collect())
}

fn chosen(apply: bool, only: &[Capability], ids: &[String]) -> Result<Vec<Check>, String> {
    let all: Vec<Check> = reads::checks().into_iter().chain(apply::checks()).collect();
    for id in ids {
        let check = all.iter().find(|check| check.id == id).ok_or_else(|| {
            let known: Vec<&str> = all.iter().map(|check| check.id).collect();
            format!("unknown check {id}; known checks: {}", known.join(", "))
        })?;
        if check.kind == Kind::Apply && !apply {
            return Err(format!("{id} changes the BMC; pass --apply to run it"));
        }
    }
    let narrowed = !only.is_empty() || !ids.is_empty();
    Ok(all
        .into_iter()
        .filter(|check| check.kind == Kind::Read || apply)
        .filter(|check| {
            !narrowed || only.contains(&check.capability) || ids.iter().any(|id| id == check.id)
        })
        .collect())
}

/// Runs the plan in order, consecutive read checks `jobs` at a time,
/// printing each result as it finishes.
pub(crate) async fn run(ctx: &Ctx, plan: Vec<Planned>, jobs: usize) -> Vec<CheckResult> {
    let mut results: Vec<CheckResult> = Vec::new();
    let mut plan = plan.into_iter().peekable();
    while let Some(first) = plan.next() {
        let mut batch = vec![gated(first, &results)];
        if batch[0].check.kind == Kind::Read {
            while let Some(next) = plan.next_if(|planned| planned.check.kind == Kind::Read) {
                batch.push(gated(next, &results));
            }
        }
        let mut running =
            futures::stream::iter(batch.into_iter().map(|planned| run_one(ctx, planned)))
                .buffered(jobs);
        while let Some(result) = running.next().await {
            report::progress(ctx, &result);
            results.push(result);
        }
    }
    results
}

/// Blocks `planned` when a check it follows failed or was blocked.
fn gated(planned: Planned, results: &[CheckResult]) -> Planned {
    if planned.skip.is_some() {
        return planned;
    }
    let blocker = planned.check.after.iter().find(|id| {
        results.iter().any(|result| {
            result.id == **id && matches!(result.outcome, Outcome::Fail(_) | Outcome::Blocked(_))
        })
    });
    match blocker {
        Some(id) => Planned {
            skip: Some(Outcome::Blocked(format!("{id} did not pass"))),
            ..planned
        },
        None => planned,
    }
}

async fn run_one(ctx: &Ctx, planned: Planned) -> CheckResult {
    let Planned { check, skip } = planned;
    let started = Instant::now();
    if check.kind == Kind::Apply {
        *ctx.tracing.lock().expect("tracing lock") = Some((check.id, started));
    }
    let outcome = match skip {
        Some(outcome) => outcome,
        None => {
            ctx.activity.start(check.id, started);
            let outcome = run_until_answered(ctx, &check, started).await;
            ctx.activity.finish(check.id);
            outcome
        }
    };
    CheckResult {
        id: check.id,
        driver: ctx.selection.driver(check.capability),
        outcome,
        elapsed: started.elapsed(),
    }
}

/// Runs `check`, running it again every [`RETRY_INTERVAL`] while it ends
/// unreachable, until [`UNREACHABLE_TIMEOUT`] passes, unless it is not safe
/// to repeat. The check's own call is the probe: iDRAC accepts logins well
/// before its other services answer.
async fn run_until_answered(ctx: &Ctx, check: &Check, started: Instant) -> Outcome {
    const UNREACHABLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
    const RETRY_INTERVAL: Duration = Duration::from_secs(30);
    let deadline = started + UNREACHABLE_TIMEOUT;
    loop {
        let reason = match tokio::time::timeout(check.timeout, (check.run)(ctx)).await {
            Ok(Ok(detail)) => return Outcome::Pass(detail),
            Ok(Err(Outcome::Unreachable(reason))) => reason,
            Ok(Err(outcome)) => return outcome,
            Err(_) => {
                if check.kind == Kind::Apply {
                    ctx.not_restored(format!(
                        "{} was cut off; anything it changed may not be restored",
                        check.id
                    ));
                }
                return Outcome::Fail(format!(
                    "did not finish within {}s",
                    check.timeout.as_secs()
                ));
            }
        };
        if !check.rerun {
            return Outcome::Fail(format!(
                "{reason}; not run again, since repeating it could redo a change"
            ));
        }
        if Instant::now() + RETRY_INTERVAL > deadline {
            return Outcome::Fail(format!(
                "{reason}; still unavailable after {}s",
                UNREACHABLE_TIMEOUT.as_secs()
            ));
        }
        ctx.trace_as(
            check.id,
            started,
            format!("{reason}; running again in {}s", RETRY_INTERVAL.as_secs()),
        );
        tokio::time::sleep(RETRY_INTERVAL).await;
    }
}

impl From<CatalogError> for Outcome {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Unsupported { .. } => Self::Unsupported(error.to_string()),
            other => Self::Fail(other.to_string()),
        }
    }
}

/// Maps a driver error from `operation` to how the check ends.
fn failed(operation: &'static str) -> impl FnOnce(PlatformError) -> Outcome {
    move |error| match error {
        PlatformError::Unsupported => {
            Outcome::Unsupported(format!("{operation}: the driver reports it unsupported"))
        }
        other if is_transient(&other) => Outcome::Unreachable(describe(operation, &other)),
        other => Outcome::Fail(describe(operation, &other)),
    }
}

/// Maps an error reading `resource` to a failure: the step needs the read
/// to verify what a driver did.
fn unreadable(resource: &'static str) -> impl FnOnce(PlatformError) -> Outcome {
    move |error| {
        let reason = describe(&format!("reading {resource}"), &error);
        if is_transient(&error) {
            Outcome::Unreachable(reason)
        } else {
            Outcome::Fail(reason)
        }
    }
}

/// Whether `error` means the BMC is not answering right now: no connection,
/// or a service that reports itself temporarily unavailable.
fn is_transient(error: &PlatformError) -> bool {
    matches!(
        error,
        PlatformError::Unreachable | PlatformError::Bmc { status: 503, .. }
    )
}

/// `operation`'s error with the HTTP status and Redfish message id up front.
fn describe(operation: &str, error: &PlatformError) -> String {
    const MAX_MESSAGE: usize = 400;
    match error {
        PlatformError::Bmc {
            status,
            message_id,
            message,
        } => {
            let truncated: String = message.chars().take(MAX_MESSAGE).collect();
            let message = if truncated.len() < message.len() {
                format!("{truncated}…")
            } else {
                truncated
            };
            format!(
                "{operation}: HTTP {status} {}: {message}",
                message_id.as_deref().unwrap_or("(no MessageId)")
            )
        }
        other => format!("{operation}: {other}"),
    }
}

/// One expected-versus-actual setting difference.
type Difference<'a> = (&'a str, &'a Value, Option<&'a Value>);

/// "in effect", or the first few expected-versus-actual differences.
fn differences<'a>(diffs: impl ExactSizeIterator<Item = Difference<'a>>) -> String {
    const SHOWN: usize = 5;
    let count = diffs.len();
    if count == 0 {
        return "in effect".to_string();
    }
    let shown: Vec<String> = diffs
        .take(SHOWN)
        .map(|(key, expected, actual)| format!("{key} ({})", expected_actual(expected, actual)))
        .collect();
    let mut text = format!(
        "{}: {}",
        counted(count, "difference", "differences"),
        shown.join("; ")
    );
    if count > SHOWN {
        text.push_str(&format!("; {} more", count - SHOWN));
    }
    text
}

/// "in effect", or a count with one field per differing setting.
fn difference_detail<'a>(diffs: impl ExactSizeIterator<Item = Difference<'a>>) -> Detail {
    let count = diffs.len();
    if count == 0 {
        return "in effect".to_string().into();
    }
    Detail {
        summary: counted(count, "difference", "differences"),
        fields: diffs
            .map(|(key, expected, actual)| (key.to_string(), expected_actual(expected, actual)))
            .collect(),
    }
}

fn expected_actual(expected: &Value, actual: Option<&Value>) -> String {
    match actual {
        Some(actual) => format!("expected {}, actual {}", plain(expected), plain(actual)),
        None => format!("expected {}, not reported", plain(expected)),
    }
}

/// A JSON value as text, without the quotes around a string.
fn plain(value: &Value) -> String {
    match value {
        Value::String(text) if text.is_empty() => "(empty)".to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `count` with `one` or `many`, as `1 entry` or `2 entries`.
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// A value the BMC may not report, as reported or `unreported`.
fn reported<T: Debug>(value: &Option<T>) -> String {
    match value {
        Some(value) => format!("{value:?}").trim_matches('"').to_string(),
        None => "unreported".to_string(),
    }
}

fn yes_no(holds: bool, yes: &str, no: &str) -> String {
    if holds { yes } else { no }.to_string()
}

/// The boot-order conditions in words; a condition the driver does not
/// manage is reported as such rather than as holding.
fn boot_order_summary(status: BootOrderStatus) -> String {
    let managed = |holds: Option<bool>, name: &str, yes: &str, no: &str| match holds {
        Some(true) => yes.to_string(),
        Some(false) => no.to_string(),
        None => format!("{name} not managed"),
    };
    [
        if status.boot_interface_first {
            "boot interface first".to_string()
        } else {
            "boot interface not first".to_string()
        },
        managed(
            status.disk_enabled,
            "disks",
            "disks enabled",
            "disks dropped",
        ),
        managed(
            status.other_network_options_disabled,
            "other network boot",
            "other network boot off",
            "other network boot on",
        ),
    ]
    .join(" · ")
}
