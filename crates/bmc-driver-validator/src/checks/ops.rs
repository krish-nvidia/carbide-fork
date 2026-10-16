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

//! Operations checks share: polling, waiting on the work a driver started,
//! host restarts and power transitions, BMC resets, and settings that take
//! effect on a restart.

use std::cell::Cell;
use std::time::{Duration, Instant};

use bmc_platform::{
    DriverOutcome, LockdownDesiredState, LockdownScope, LockdownState, LockdownStatus,
    OperationReference, OperationStatus, PlatformError,
};
use bmc_runtime::ConnectedBmc;
use carbide_redfish::nv_redfish::RedfishBmc;
use nv_redfish::resource::{PowerState, ResetType};

use super::{Ctx, Outcome, Step, describe, failed};
use crate::connect::Target;

/// How long accepted work may stay queued or running before or after a reset.
const JOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const JOB_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How long a power transition may take to show in the reported state.
const POWER_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How long settings may take to read back as in effect after a restart,
/// while the host POSTs.
pub(super) const SETTLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub(super) const SETTLE_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// How long a change that needs no restart may take to read back.
pub(super) const IMMEDIATE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const QUICK_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How long a BMC that accepted an AC power cycle and started no work may
/// take to lose power; a BMC can accept the request well before it cuts it.
const AC_CYCLE_DOWN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// How long a BMC may refuse a lockdown change before taking it, as some do
/// for minutes after earlier setup steps; the machine controller retries
/// until it takes.
const LOCKDOWN_REFUSAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const LOCKDOWN_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// How long a reset BMC may keep answering, and then take to answer again.
const BMC_DOWN_TIMEOUT: Duration = Duration::from_secs(2 * 60);
pub(super) const BMC_UP_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const BMC_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// One attempt of a poll.
pub(super) enum Attempt<T> {
    Done(T),
    /// Not there yet; the reason is reported if the poll times out.
    NotYet(String),
    Failed(Outcome),
}

/// Retries `attempt` every `interval` until it is done, fails, or
/// `timeout` passes, tracing each reading that is not there yet.
pub(super) async fn poll<T>(
    ctx: &Ctx,
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
        ctx.trace(format!("waiting: {reason}"));
        if Instant::now() + interval > deadline {
            return Err(Outcome::Fail(format!(
                "{reason} after {}s",
                timeout.as_secs()
            )));
        }
        tokio::time::sleep(interval).await;
    }
}

/// Polls `reached` briefly, then, unless a restart already happened,
/// restarts the host and polls while it POSTs: settings staged for the BIOS
/// take effect on a reset even when the driver reports them complete.
/// Returns what `reached` saw and whether a restart happened.
pub(super) async fn converge<T>(
    ctx: &Ctx,
    restarted: bool,
    reached: impl AsyncFn() -> Attempt<T>,
) -> Result<(T, bool), Outcome> {
    if !restarted {
        match poll(ctx, IMMEDIATE_TIMEOUT, QUICK_POLL_INTERVAL, async || {
            reached().await
        })
        .await
        {
            Ok(value) => return Ok((value, false)),
            Err(Outcome::Fail(reason)) => ctx.trace(format!(
                "{reason}; restarting the host in case it applies on reset"
            )),
            Err(other) => return Err(other),
        }
        restart_host(ctx).await?;
    }
    let value = poll(ctx, SETTLE_TIMEOUT, SETTLE_POLL_INTERVAL, async || {
        reached().await
    })
    .await?;
    Ok((value, true))
}

/// Settles `outcome`, then restarts the host unless settling already did,
/// so a change staged for the next reset takes effect.
pub(super) async fn take_effect(ctx: &Ctx, outcome: &DriverOutcome) -> Result<(), Outcome> {
    if !settle(ctx, outcome).await? {
        ctx.trace("restarting the host so the change takes effect");
        restart_host(ctx).await?;
    }
    Ok(())
}

/// Waits for an outcome's work to finish, restarting the host once when
/// some of it runs on the next reset; returns whether it restarted.
pub(super) async fn settle(ctx: &Ctx, outcome: &DriverOutcome) -> Result<bool, Outcome> {
    let mut awaiting_reset = false;
    for reference in outcome.references() {
        let status = poll(ctx, JOB_TIMEOUT, JOB_POLL_INTERVAL, async || {
            match ctx.bmc.operation_status(reference).await {
                Ok(OperationStatus::Running) => {
                    Attempt::NotYet(format!("{} is still running", reference.uri()))
                }
                Ok(status) => Attempt::Done(status),
                Err(error) => Attempt::NotYet(describe("operation status", &error)),
            }
        })
        .await?;
        ctx.trace(format!("{}: {status:?}", reference.uri()));
        match status {
            OperationStatus::AwaitingReset => awaiting_reset = true,
            OperationStatus::Completed | OperationStatus::Running => {}
            ended => return Err(ended_outcome(reference, ended)),
        }
    }
    if awaiting_reset {
        ctx.trace("work runs on the next host reset; restarting the host");
        restart_host(ctx).await?;
        wait_for_operations(ctx, outcome).await?;
    }
    Ok(awaiting_reset)
}

/// Waits until every reference in `outcome` completes.
pub(super) async fn wait_for_operations(ctx: &Ctx, outcome: &DriverOutcome) -> Result<(), Outcome> {
    for reference in outcome.references() {
        poll(ctx, JOB_TIMEOUT, JOB_POLL_INTERVAL, async || {
            match ctx.bmc.operation_status(reference).await {
                Ok(OperationStatus::Completed) => Attempt::Done(()),
                Ok(status @ (OperationStatus::Running | OperationStatus::AwaitingReset)) => {
                    Attempt::NotYet(format!("{} is {status:?}", reference.uri()))
                }
                Ok(ended) => Attempt::Failed(ended_outcome(reference, ended)),
                Err(error) => Attempt::NotYet(describe("operation status", &error)),
            }
        })
        .await?;
        ctx.trace(format!("{}: Completed", reference.uri()));
    }
    Ok(())
}

/// Restarts the host through the selected power driver, or powers it on
/// when it is off, and waits until it reports on.
pub(super) async fn restart_host(ctx: &Ctx) -> Result<(), Outcome> {
    let reset = match power_state(&ctx.bmc).await.map_err(failed("state"))? {
        Some(PowerState::Off) => ResetType::On,
        _ => ResetType::ForceRestart,
    };
    set_power(ctx, &ctx.bmc, reset).await.map(drop)
}

/// The host power state through `bmc`'s power driver, read afresh.
pub(super) async fn power_state(
    bmc: &ConnectedBmc<RedfishBmc>,
) -> Result<Option<PowerState>, PlatformError> {
    bmc.power()
        .map_err(|_| PlatformError::Unsupported)?
        .state()
        .await
}

/// Requests `reset` through `bmc`'s power driver and waits for the state it
/// leads to.
pub(super) async fn set_power(ctx: &Ctx, bmc: &ConnectedBmc<RedfishBmc>, reset: ResetType) -> Step {
    let target = match reset {
        ResetType::ForceOff | ResetType::GracefulShutdown => PowerState::Off,
        _ => PowerState::On,
    };
    let started = Instant::now();
    let outcome = bmc.power()?.set(reset).await.map_err(failed("set"))?;
    ctx.trace(format!("power {reset:?}: {}", how(&outcome)));
    wait_for_operations(ctx, &outcome).await?;
    wait_for_power(ctx, bmc, target).await?;
    ctx.trace(format!("power reads {target:?}"));
    Ok(format!("{reset:?} reached {target:?} in {}", took(started)))
}

pub(super) async fn wait_for_power(
    ctx: &Ctx,
    bmc: &ConnectedBmc<RedfishBmc>,
    target: PowerState,
) -> Result<(), Outcome> {
    poll(
        ctx,
        POWER_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match power_state(bmc).await {
            Ok(Some(state)) if state == target => Attempt::Done(()),
            Ok(state) => Attempt::NotYet(format!("host reports power {state:?}, not {target:?}")),
            Err(error) => Attempt::NotYet(describe("power state", &error)),
        },
    )
    .await
}

/// Resets the BMC, waits for it to go down and come back, and requires
/// rediscovery to select the same drivers.
pub(super) async fn reset_bmc(ctx: &Ctx) -> Step {
    let started = Instant::now();
    let outcome = sent_or_dropped(ctx, "reset", ctx.bmc.bmc_control()?.reset().await)?;
    ctx.trace(format!("reset: {}", how(&outcome)));
    wait_for_bmc_down(ctx, BMC_DOWN_TIMEOUT).await?;
    wait_for_login(ctx, &ctx.target, BMC_UP_TIMEOUT).await?;
    let down_and_up = took(started);
    // A BMC can accept logins before it serves every resource discovery reads.
    let identity = poll(ctx, BMC_UP_TIMEOUT, BMC_POLL_INTERVAL, async || {
        match ctx.target.discover().await {
            Ok(identity) => Attempt::Done(identity),
            Err(reason) => Attempt::NotYet(format!("rediscovery fails: {reason}")),
        }
    })
    .await?;
    if !ctx
        .selection
        .resolves_same(&identity)
        .map_err(Outcome::Fail)?
    {
        return Err(Outcome::Fail(
            "after the reset the BMC's identity selects different drivers".to_string(),
        ));
    }
    Ok(format!(
        "answered again {down_and_up} after the reset ({}); rediscovery selects the same drivers",
        how(&outcome)
    ))
}

/// The outcome of a request that restarts the BMC, which can go down before
/// it answers: an unreachable BMC then means the request was sent, and the
/// caller waits for the BMC to come back.
pub(super) fn sent_or_dropped(
    ctx: &Ctx,
    operation: &'static str,
    result: Result<DriverOutcome, PlatformError>,
) -> Result<DriverOutcome, Outcome> {
    match result {
        Err(PlatformError::Unreachable) => {
            ctx.trace(format!(
                "{operation}: the BMC dropped before answering; waiting for it"
            ));
            Ok(DriverOutcome::complete())
        }
        other => other.map_err(failed(operation)),
    }
}

/// Leaves the host on: powers it on when it reads off, otherwise waits for
/// on. Reads are retried while the BMC is still coming back.
pub(super) async fn power_on(ctx: &Ctx) -> Result<(), Outcome> {
    let state = poll(
        ctx,
        POWER_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match power_state(&ctx.bmc).await {
            Ok(state) => Attempt::Done(state),
            Err(error) => Attempt::NotYet(describe("power state", &error)),
        },
    )
    .await?;
    match state {
        Some(PowerState::Off) => set_power(ctx, &ctx.bmc, ResetType::On).await.map(drop),
        _ => wait_for_power(ctx, &ctx.bmc, PowerState::On).await,
    }
}

/// Waits for the BMC to stop answering, as after it resets or loses power,
/// failing after `timeout`.
pub(super) async fn wait_for_bmc_down(ctx: &Ctx, timeout: Duration) -> Result<(), Outcome> {
    poll(ctx, timeout, QUICK_POLL_INTERVAL, async || {
        match ctx.target.log_in().await {
            Ok(()) | Err(PlatformError::Auth(_)) => {
                Attempt::NotYet("the BMC still answers".to_string())
            }
            Err(_) => Attempt::Done(()),
        }
    })
    .await?;
    ctx.trace("the BMC stopped answering");
    Ok(())
}

/// How long the BMC may take to lose power after accepting an AC power cycle
/// that produced `outcome`. A request that started work cuts power only once
/// that work runs, which can wait for a host restart.
pub(super) fn ac_cycle_down_timeout(outcome: &DriverOutcome) -> Duration {
    if outcome.references().next().is_some() {
        JOB_TIMEOUT
    } else {
        AC_CYCLE_DOWN_TIMEOUT
    }
}

/// Waits until `target` logs in.
pub(super) async fn wait_for_login(
    ctx: &Ctx,
    target: &Target,
    timeout: Duration,
) -> Result<(), Outcome> {
    poll(ctx, timeout, BMC_POLL_INTERVAL, async || {
        match target.log_in().await {
            Ok(()) => Attempt::Done(()),
            Err(error) => Attempt::NotYet(format!("logging in fails: {error}")),
        }
    })
    .await
}

/// Sets lockdown, settles, and converges on the driver's aggregate reading
/// `desired`, as controllers decide; the halves are reported as read.
pub(super) async fn set_lockdown(ctx: &Ctx, desired: LockdownDesiredState) -> Step {
    let lockdown = ctx.bmc.lockdown()?;
    let target = match desired {
        LockdownDesiredState::Enabled => LockdownState::Enabled,
        LockdownDesiredState::Disabled => LockdownState::Disabled,
    };
    let reached = |status: &LockdownStatus| status.aggregate == target;
    let halves = |status: &LockdownStatus| {
        format!(
            "{:?} · host {:?} · bmc {:?}",
            status.aggregate, status.host, status.bmc
        )
    };
    let status = lockdown.status().await.map_err(failed("status"))?;
    if reached(&status) {
        return Ok(format!("already {}", halves(&status)));
    }
    ctx.trace(format!("lockdown reads {}", halves(&status)));
    let started = Instant::now();
    let refusals = Cell::new(0);
    let outcome = poll(
        ctx,
        LOCKDOWN_REFUSAL_TIMEOUT,
        LOCKDOWN_RETRY_INTERVAL,
        async || match lockdown.set(LockdownScope::All, desired).await {
            Ok(outcome) => Attempt::Done(outcome),
            Err(
                error @ PlatformError::Bmc {
                    status: 400..=499, ..
                },
            ) => {
                refusals.set(refusals.get() + 1);
                Attempt::NotYet(describe("set", &error))
            }
            Err(error) => Attempt::Failed(failed("set")(error)),
        },
    )
    .await?;
    ctx.trace(format!("set lockdown {desired:?}: {}", how(&outcome)));
    let refused = match refusals.get() {
        0 => String::new(),
        count => format!(
            ", after the BMC refused it {count} times over {}",
            took(started)
        ),
    };
    let restarted = settle(ctx, &outcome).await?;
    let (status, restarted) = converge(ctx, restarted, async || match lockdown.status().await {
        Ok(status) if reached(&status) => Attempt::Done(status),
        Ok(status) => Attempt::NotYet(format!("lockdown reads {}", halves(&status))),
        Err(error) => Attempt::NotYet(describe("status", &error)),
    })
    .await?;
    Ok(format!(
        "{} in {} ({}{}{refused})",
        halves(&status),
        took(started),
        how(&outcome),
        if restarted {
            ", after a host restart"
        } else {
            ""
        }
    ))
}

/// How the driver says a request completed: at once, or by starting work.
pub(super) fn how(outcome: &DriverOutcome) -> String {
    match outcome {
        DriverOutcome::Complete => "applied at once".to_string(),
        accepted => format!("started {}", uris(accepted)),
    }
}

pub(super) fn took(started: Instant) -> String {
    format!("{:.1}s", started.elapsed().as_secs_f64())
}

pub(super) fn uris(outcome: &DriverOutcome) -> String {
    outcome
        .references()
        .map(|reference| reference.uri().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn ended_outcome(reference: &OperationReference, status: OperationStatus) -> Outcome {
    Outcome::Fail(match status {
        OperationStatus::Failed { state, message } => format!(
            "{} ended {state}: {}",
            reference.uri(),
            message.unwrap_or_default()
        ),
        OperationStatus::NeedsIntervention { state } => {
            format!("{} waits for an operator ({state})", reference.uri())
        }
        other => format!("{} is {other:?}", reference.uri()),
    })
}
