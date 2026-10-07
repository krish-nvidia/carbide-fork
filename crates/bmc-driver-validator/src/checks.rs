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
//! A check is one entry in [`reads::checks`] or [`apply::checks`]: an id,
//! the capability whose selected driver it exercises, and an async function
//! that calls that driver through the connected BMC the way a controller
//! does. Read checks run concurrently; apply checks run one at a time in
//! registry order, after the reads.

mod apply;
mod reads;

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bmc_drivers::CatalogError;
use bmc_platform::{
    BiosSettings, BootInterfaceSelector, BootOrderStatus, Capability, PlatformError,
};
use bmc_runtime::ConnectedBmc;
use carbide_redfish::nv_redfish::{NvRedfishClientPool, RedfishBmc};
use futures::StreamExt;
use serde_json::Value;

use crate::report::{self, Redactor};
use crate::selection::Selection;

/// What the operator supplied for checks to use.
pub(crate) struct Inputs {
    /// The account this run logged in with.
    pub(crate) username: String,
    /// Whether IPMI is attached to the connected BMC.
    pub(crate) ipmi: bool,
    pub(crate) boot_interface: Option<BootInterfaceSelector>,
    pub(crate) bios_profile: BiosSettings,
}

/// State shared by every check of one run.
pub(crate) struct Ctx {
    pub(crate) bmc: ConnectedBmc<RedfishBmc>,
    pool: Arc<NvRedfishClientPool>,
    address: SocketAddr,
    inputs: Inputs,
    pub(crate) redactor: Arc<Redactor>,
    /// Changes a check made and could not undo.
    not_restored: Mutex<Vec<String>>,
}

impl Ctx {
    pub(crate) fn new(
        bmc: ConnectedBmc<RedfishBmc>,
        pool: Arc<NvRedfishClientPool>,
        address: SocketAddr,
        inputs: Inputs,
        redactor: Arc<Redactor>,
    ) -> Self {
        Self {
            bmc,
            pool,
            address,
            inputs,
            redactor,
            not_restored: Mutex::new(Vec::new()),
        }
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
    Pass(String),
    Fail(String),
    /// The selection or the driver reports the operation unsupported.
    Unsupported(String),
    /// The check could not run: a missing input or a failed prerequisite.
    Skipped(String),
}

/// A check's result: the detail it passed with, or how else it ended.
pub(crate) type Step = Result<String, Outcome>;

type CheckFn = for<'a> fn(&'a Ctx) -> Pin<Box<dyn Future<Output = Step> + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// Only reads the BMC.
    Read,
    /// Changes the BMC or host; needs `--apply`.
    Apply,
}

pub(crate) struct Check {
    pub(crate) id: &'static str,
    pub(crate) capability: Capability,
    pub(crate) kind: Kind,
    needs_boot_interface: bool,
    timeout: Duration,
    run: CheckFn,
}

impl Check {
    fn read(id: &'static str, capability: Capability, run: CheckFn) -> Self {
        Self {
            id,
            capability,
            kind: Kind::Read,
            needs_boot_interface: false,
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

    fn needs_boot_interface(self) -> Self {
        Self {
            needs_boot_interface: true,
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

/// Picks the checks to run: every read check, plus the apply checks with
/// `apply`, narrowed to the capabilities in `only` and the checks in `ids`.
pub(crate) fn plan(
    selection: &Selection,
    apply: bool,
    only: &[Capability],
    ids: &[String],
    have_boot_interface: bool,
) -> Result<Vec<Planned>, String> {
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
        .map(|check| {
            let skip = if !selection.is_supported(check.capability) {
                Some(Outcome::Unsupported(selection.reason(check.capability)))
            } else if check.needs_boot_interface && !have_boot_interface {
                Some(Outcome::Skipped("needs --boot-mac".to_string()))
            } else {
                None
            };
            Planned { check, skip }
        })
        .collect())
}

/// Runs the read checks `jobs` at a time, then the apply checks in order,
/// printing each result as it finishes.
pub(crate) async fn run(
    ctx: &Ctx,
    plan: Vec<Planned>,
    selection: &Selection,
    jobs: usize,
) -> Vec<CheckResult> {
    let (reads, applies): (Vec<_>, Vec<_>) = plan
        .into_iter()
        .partition(|planned| planned.check.kind == Kind::Read);
    let mut results = Vec::new();
    let mut running = futures::stream::iter(
        reads
            .into_iter()
            .map(|planned| run_one(ctx, planned, selection)),
    )
    .buffered(jobs);
    while let Some(result) = running.next().await {
        report::progress(ctx, &result);
        results.push(result);
    }
    for planned in applies {
        let result = run_one(ctx, planned, selection).await;
        report::progress(ctx, &result);
        results.push(result);
    }
    results
}

async fn run_one(ctx: &Ctx, planned: Planned, selection: &Selection) -> CheckResult {
    let Planned { check, skip } = planned;
    let started = Instant::now();
    let outcome = match skip {
        Some(outcome) => outcome,
        None => match tokio::time::timeout(check.timeout, (check.run)(ctx)).await {
            Ok(Ok(detail)) => Outcome::Pass(detail),
            Ok(Err(outcome)) => outcome,
            Err(_) => {
                if check.kind == Kind::Apply {
                    ctx.not_restored(format!(
                        "{} was cut off; anything it changed may not be restored",
                        check.id
                    ));
                }
                Outcome::Fail(format!(
                    "did not finish within {}s",
                    check.timeout.as_secs()
                ))
            }
        },
    };
    CheckResult {
        id: check.id,
        driver: selection.driver(check.capability),
        outcome,
        elapsed: started.elapsed(),
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
        other => Outcome::Fail(describe(operation, &other)),
    }
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

/// "in effect", or the first few expected-versus-actual differences.
fn differences<'a>(
    diffs: impl ExactSizeIterator<Item = (&'a str, &'a Value, Option<&'a Value>)>,
) -> String {
    const SHOWN: usize = 5;
    let count = diffs.len();
    if count == 0 {
        return "in effect".to_string();
    }
    let shown: Vec<String> = diffs
        .take(SHOWN)
        .map(|(key, expected, actual)| match actual {
            Some(actual) => format!("{key} (expected {expected}, actual {actual})"),
            None => format!("{key} (expected {expected}, not reported)"),
        })
        .collect();
    let mut text = format!("{count} differences: {}", shown.join("; "));
    if count > SHOWN {
        text.push_str(&format!("; {} more", count - SHOWN));
    }
    text
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

/// One attempt of a poll.
enum Attempt<T> {
    Done(T),
    /// Not there yet; the reason is reported if the poll times out.
    NotYet(String),
    Failed(Outcome),
}

/// Retries `attempt` every `interval` until it is done, fails, or
/// `timeout` passes.
async fn poll<T>(
    timeout: Duration,
    interval: Duration,
    mut attempt: impl AsyncFnMut() -> Attempt<T>,
) -> Result<T, Outcome> {
    let deadline = Instant::now() + timeout;
    loop {
        let reason = match attempt().await {
            Attempt::Done(value) => return Ok(value),
            Attempt::Failed(outcome) => return Err(outcome),
            Attempt::NotYet(reason) => reason,
        };
        if Instant::now() + interval > deadline {
            return Err(Outcome::Fail(format!(
                "{reason} after {}s",
                timeout.as_secs()
            )));
        }
        tokio::time::sleep(interval).await;
    }
}
