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

//! Checks that change the BMC or host and verify the effect.
//!
//! BIOS-backed settings take effect on a host reset, so each settings check
//! applies, restarts the host once, waits for the BMC's job, and then polls
//! the driver's status until the settings are in effect. iDRAC drops every
//! queued job on each BIOS write, so settings checks do not share a restart.
//! Power, lockdown, and account checks restore what they change.

use std::time::Duration;

use bmc_platform::{
    Capability, ClassifyBmcError, ConsoleState, DriverOutcome, Fetched, LockdownDesiredState,
    LockdownScope, LockdownState, OperationReference, OperationStatus, PlatformError, Quirk,
};
use carbide_secrets::credentials::Credentials;
use nv_redfish::Bmc;
use nv_redfish::account::ManagerAccountCreate;
use nv_redfish::bmc_http::BmcCredentials;
use nv_redfish::core::ODataId;
use nv_redfish::resource::{PowerState, ResetType};
use serde_json::{Map, Value};

use super::{Attempt, Check, Ctx, Outcome, Step, describe, differences, failed, poll};

/// How long accepted work may stay queued or running before or after a reset.
const JOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const JOB_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How long a power transition may take to show in the reported state.
const POWER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POWER_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How long settings may take to read back as in effect after a restart,
/// while the host POSTs.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SETTLE_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// How long a setting that needs no restart may take to read back.
const IMMEDIATE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a new or deleted account may take to affect logins.
const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn checks() -> Vec<Check> {
    vec![
        Check::apply("bios.apply", Capability::Bios, |ctx| {
            Box::pin(bios_apply(ctx))
        }),
        Check::apply("boot_order.configure", Capability::BootOrder, |ctx| {
            Box::pin(boot_order_configure(ctx))
        })
        .needs_boot_interface(),
        Check::apply("console.setup", Capability::Console, |ctx| {
            Box::pin(console_setup(ctx))
        }),
        Check::apply("power.cycle", Capability::Power, |ctx| {
            Box::pin(power_cycle(ctx))
        }),
        Check::apply("accounts.lifecycle", Capability::Accounts, |ctx| {
            Box::pin(accounts_lifecycle(ctx))
        }),
        Check::apply("lockdown.toggle", Capability::Lockdown, |ctx| {
            Box::pin(lockdown_toggle(ctx))
        }),
    ]
}

async fn bios_apply(ctx: &Ctx) -> Step {
    let bios = ctx.bmc.drivers().bios()?;
    let profile = &ctx.inputs.bios_profile;
    let boot_interface = ctx.inputs.boot_interface.as_ref();
    apply_until_in_effect(
        ctx,
        async || {
            let status = bios
                .status(&ctx.bmc.operation_context(), profile, boot_interface)
                .await
                .map_err(failed("status"))?;
            Ok((!status.is_applied).then(|| {
                differences(
                    status
                        .differences
                        .iter()
                        .map(|diff| (diff.key.as_str(), &diff.expected, diff.actual.as_ref())),
                )
            }))
        },
        async || {
            bios.apply(&ctx.bmc.operation_context(), profile, boot_interface)
                .await
                .map_err(failed("apply"))
        },
    )
    .await
}

async fn boot_order_configure(ctx: &Ctx) -> Step {
    let boot_order = ctx.bmc.drivers().boot_order()?;
    let selector = ctx
        .inputs
        .boot_interface
        .as_ref()
        .ok_or_else(|| Outcome::Skipped("needs --boot-mac".to_string()))?;
    apply_until_in_effect(
        ctx,
        async || {
            let status = boot_order
                .status(&ctx.bmc.operation_context(), selector)
                .await
                .map_err(failed("status"))?;
            Ok((!status.is_configured()).then(|| format!("{status:?}")))
        },
        async || {
            boot_order
                .configure(&ctx.bmc.operation_context(), selector)
                .await
                .map_err(failed("configure"))
        },
    )
    .await
}

async fn console_setup(ctx: &Ctx) -> Step {
    let console = ctx.bmc.drivers().console()?;
    apply_until_in_effect(
        ctx,
        async || {
            let status = console
                .status(&ctx.bmc.operation_context())
                .await
                .map_err(failed("status"))?;
            Ok((status.state != ConsoleState::Enabled)
                .then(|| format!("{:?} ({})", status.state, status.message)))
        },
        async || {
            console
                .setup(&ctx.bmc.operation_context())
                .await
                .map_err(failed("setup"))
        },
    )
    .await
}

/// Applies settings and verifies they take effect: `pending` reads why the
/// settings are not yet in effect, or `None` once they are. Settings
/// already in effect are applied once more, which must start no work.
async fn apply_until_in_effect(
    ctx: &Ctx,
    pending: impl AsyncFn() -> Result<Option<String>, Outcome>,
    apply: impl AsyncFn() -> Result<DriverOutcome, Outcome>,
) -> Step {
    if pending().await?.is_none() {
        return reapply_changes_nothing(ctx, &apply, "already in effect").await;
    }
    let outcome = apply().await?;
    let restarted = settle(ctx, &outcome).await?;
    if !restarted {
        restart_host(ctx).await?;
    }
    poll(
        SETTLE_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match pending().await {
            Ok(None) => Attempt::Done(()),
            Ok(Some(reason)) => Attempt::NotYet(format!("not in effect: {reason}")),
            // The BMC can refuse BIOS reads while the host POSTs.
            Err(Outcome::Fail(reason)) => Attempt::NotYet(reason),
            Err(other) => Attempt::Failed(other),
        },
    )
    .await?;
    reapply_changes_nothing(ctx, &apply, "in effect after a host restart").await
}

async fn reapply_changes_nothing(
    ctx: &Ctx,
    apply: &impl AsyncFn() -> Result<DriverOutcome, Outcome>,
    state: &str,
) -> Step {
    match apply().await? {
        DriverOutcome::Complete => Ok(format!("{state}; applying again changed nothing")),
        accepted => {
            let uris = uris(&accepted);
            ctx.not_restored(format!(
                "work staged by a repeated apply is still pending: {uris}"
            ));
            Err(Outcome::Fail(format!(
                "{state}, yet applying again started {uris}"
            )))
        }
    }
}

/// Cycles host power away from its current state and back.
async fn power_cycle(ctx: &Ctx) -> Step {
    let Some(original) = power_state(ctx).await.map_err(failed("state"))? else {
        return Err(Outcome::Skipped(
            "the platform reports no single power state".to_string(),
        ));
    };
    let (away, back) = match original {
        PowerState::Off => (ResetType::On, ResetType::ForceOff),
        _ => (ResetType::ForceOff, ResetType::On),
    };
    let away_result = set_power(ctx, away).await;
    let back_result = set_power(ctx, back).await;
    match power_state(ctx).await {
        Ok(Some(state)) if state == original => {}
        current => ctx.not_restored(format!(
            "host power was {original:?}; it now reads {current:?}"
        )),
    }
    Ok(format!("{}; {}", away_result?, back_result?))
}

/// Creates a temporary account, logs in with it, deletes it, and confirms
/// the login stops working; the run's own account is never changed.
async fn accounts_lifecycle(ctx: &Ctx) -> Step {
    let accounts = ctx.bmc.drivers().accounts()?;
    let username = format!("nicoval{:04}", rand::random::<u16>() % 10_000);
    let password = Credentials::generate_password();
    ctx.redactor.add(&password);

    if listed(ctx, &username).await? {
        return Err(Outcome::Skipped(format!(
            "an account named {username} already exists"
        )));
    }
    let created = accounts
        .create(
            &ctx.bmc.operation_context(),
            ManagerAccountCreate::builder(
                password.clone(),
                username.clone(),
                "Administrator".to_string(),
            ),
        )
        .await;
    let verified = match created {
        Ok(outcome) => verify_new_account(ctx, &outcome, &username, &password).await,
        Err(error) => Err(failed("create")(error)),
    };
    if !listed(ctx, &username).await.unwrap_or(true) {
        return verified;
    }

    let deleted = accounts
        .delete(&ctx.bmc.operation_context(), &username)
        .await
        .map_err(failed("delete"));
    let gone = match deleted {
        Ok(outcome) => verify_account_gone(ctx, &outcome, &username, &password).await,
        Err(outcome) => Err(outcome),
    };
    if gone.is_err() {
        ctx.not_restored(format!("temporary account {username} may still exist"));
    }
    Ok(format!("{}; {}", verified?, gone?))
}

async fn verify_new_account(
    ctx: &Ctx,
    outcome: &DriverOutcome,
    username: &str,
    password: &str,
) -> Step {
    wait_for_operations(ctx, outcome).await?;
    if !listed(ctx, username).await? {
        return Err(Outcome::Fail(format!(
            "created {username}, but it is not listed"
        )));
    }
    poll(
        ACCOUNT_TIMEOUT,
        POWER_POLL_INTERVAL,
        async || match log_in(ctx, username, password).await {
            Ok(()) => Attempt::Done(()),
            Err(error) => Attempt::NotYet(format!("logging in as {username} fails: {error}")),
        },
    )
    .await?;
    Ok(format!("created {username} and logged in with it"))
}

async fn verify_account_gone(
    ctx: &Ctx,
    outcome: &DriverOutcome,
    username: &str,
    password: &str,
) -> Step {
    wait_for_operations(ctx, outcome).await?;
    if listed(ctx, username).await? {
        return Err(Outcome::Fail(format!(
            "deleted {username}, but it is still listed"
        )));
    }
    poll(ACCOUNT_TIMEOUT, POWER_POLL_INTERVAL, async || {
        ctx.pool.invalidate_service_roots_for_bmc(ctx.address);
        match log_in(ctx, username, password).await {
            Err(PlatformError::Auth(_)) => Attempt::Done(()),
            Err(error) => Attempt::NotYet(format!(
                "logging in as {username} fails with {error}, not an auth error"
            )),
            Ok(()) => Attempt::NotYet(format!("{username} can still log in")),
        }
    })
    .await?;
    Ok("deleted it, and its login is refused".to_string())
}

async fn listed(ctx: &Ctx, username: &str) -> Result<bool, Outcome> {
    let accounts = ctx
        .bmc
        .drivers()
        .accounts()?
        .list(&ctx.bmc.operation_context())
        .await
        .map_err(failed("list"))?;
    Ok(accounts
        .iter()
        .any(|account| account.user_name.as_deref() == Some(username)))
}

/// Reads an authenticated resource as `username` over a new connection.
async fn log_in(ctx: &Ctx, username: &str, password: &str) -> Result<(), PlatformError> {
    let connection = ctx
        .pool
        .connection_with_bmc_credentials(
            ctx.address,
            BmcCredentials::new(username.to_string(), password.to_string()),
        )
        .await
        .map_err(PlatformError::from_redfish)?;
    connection
        .bmc
        .get::<Fetched<Map<String, Value>>>(&ODataId::from("/redfish/v1/Managers".to_string()))
        .await
        .map(drop)
        .map_err(ClassifyBmcError::classify)
}

/// Locks down a host that is not locked down, or unlocks one that is, then
/// restores it.
async fn lockdown_toggle(ctx: &Ctx) -> Step {
    let lockdown = ctx.bmc.drivers().lockdown()?;
    let original = lockdown
        .status(&ctx.bmc.operation_context())
        .await
        .map_err(failed("status"))?
        .aggregate;
    let (away, back) = match original {
        LockdownState::Enabled => (
            LockdownDesiredState::Disabled,
            LockdownDesiredState::Enabled,
        ),
        _ => (
            LockdownDesiredState::Enabled,
            LockdownDesiredState::Disabled,
        ),
    };
    let away_result = set_lockdown(ctx, away).await;
    let back_result = set_lockdown(ctx, back).await;
    match lockdown.status(&ctx.bmc.operation_context()).await {
        Ok(status) if status.aggregate == original => {}
        current => ctx.not_restored(format!(
            "lockdown was {original:?}; it now reads {:?}",
            current.map(|status| status.aggregate)
        )),
    }
    Ok(format!("{}; {}", away_result?, back_result?))
}

async fn set_lockdown(ctx: &Ctx, desired: LockdownDesiredState) -> Step {
    let lockdown = ctx.bmc.drivers().lockdown()?;
    let expected = match desired {
        LockdownDesiredState::Enabled => LockdownState::Enabled,
        LockdownDesiredState::Disabled => LockdownState::Disabled,
    };
    let outcome = lockdown
        .set(&ctx.bmc.operation_context(), LockdownScope::All, desired)
        .await
        .map_err(failed("set"))?;
    let mut restarted = settle(ctx, &outcome).await?;
    let reached = async |timeout, interval| {
        poll(timeout, interval, async || {
            match lockdown.status(&ctx.bmc.operation_context()).await {
                Ok(status) if status.aggregate == expected => Attempt::Done(()),
                Ok(status) => Attempt::NotYet(format!(
                    "status reports {:?} ({})",
                    status.aggregate, status.message
                )),
                Err(error) => Attempt::NotYet(describe("status", &error)),
            }
        })
        .await
    };
    // Lockdown staged as BIOS settings takes effect on a reset even when the
    // driver reports it complete.
    if reached(IMMEDIATE_TIMEOUT, POWER_POLL_INTERVAL)
        .await
        .is_err()
        && !restarted
    {
        restart_host(ctx).await?;
        restarted = true;
    }
    if restarted {
        reached(SETTLE_TIMEOUT, SETTLE_POLL_INTERVAL).await?;
    }
    Ok(format!(
        "reached {expected:?}{}",
        if restarted {
            " after a host restart"
        } else {
            ""
        }
    ))
}

/// The host power state through the selected driver, read afresh.
async fn power_state(ctx: &Ctx) -> Result<Option<PowerState>, PlatformError> {
    ctx.bmc
        .drivers()
        .power()
        .map_err(|_| PlatformError::Unsupported)?
        .state(&ctx.bmc.operation_context())
        .await
}

/// Requests `reset` through the selected driver and waits for the state it
/// leads to.
async fn set_power(ctx: &Ctx, reset: ResetType) -> Step {
    let target = match reset {
        ResetType::ForceOff | ResetType::GracefulShutdown => PowerState::Off,
        _ => PowerState::On,
    };
    let outcome = ctx
        .bmc
        .drivers()
        .power()?
        .set(&ctx.bmc.operation_context(), reset)
        .await
        .map_err(failed("set"))?;
    wait_for_operations(ctx, &outcome).await?;
    wait_for_power(ctx, target).await?;
    Ok(format!("{reset:?} reached {target:?}"))
}

async fn wait_for_power(ctx: &Ctx, target: PowerState) -> Result<(), Outcome> {
    poll(
        POWER_TIMEOUT,
        POWER_POLL_INTERVAL,
        async || match power_state(ctx).await {
            Ok(Some(state)) if state == target => Attempt::Done(()),
            Ok(state) => Attempt::NotYet(format!("host reports power {state:?}, not {target:?}")),
            Err(error) => Attempt::NotYet(describe("power state", &error)),
        },
    )
    .await
}

/// Restarts the host through the selected power driver, or powers it on
/// when it is off, and waits until it reports on.
async fn restart_host(ctx: &Ctx) -> Result<(), Outcome> {
    let restarts_over_ipmi = ctx
        .bmc
        .endpoint()
        .selection()
        .quirks
        .contains(&Quirk::RedfishRestartCutsDpuPower);
    if restarts_over_ipmi && !ctx.inputs.ipmi {
        return Err(Outcome::Skipped(
            "this platform restarts its host over IPMI; pass --ipmi".to_string(),
        ));
    }
    let reset = match power_state(ctx).await.map_err(failed("state"))? {
        Some(PowerState::Off) => ResetType::On,
        _ => ResetType::ForceRestart,
    };
    set_power(ctx, reset).await.map(drop)
}

/// Waits for an outcome's work to finish, restarting the host once when
/// some of it runs on the next reset; returns whether it restarted.
async fn settle(ctx: &Ctx, outcome: &DriverOutcome) -> Result<bool, Outcome> {
    let mut awaiting_reset = false;
    for reference in outcome.references() {
        let status = poll(JOB_TIMEOUT, JOB_POLL_INTERVAL, async || {
            match ctx.bmc.operation_status(reference).await {
                Ok(OperationStatus::Running) => {
                    Attempt::NotYet(format!("{} is still running", reference.uri()))
                }
                Ok(status) => Attempt::Done(status),
                Err(error) => Attempt::NotYet(describe("operation status", &error)),
            }
        })
        .await?;
        match status {
            OperationStatus::AwaitingReset => awaiting_reset = true,
            OperationStatus::Completed | OperationStatus::Running => {}
            ended => return Err(ended_outcome(reference, ended)),
        }
    }
    if awaiting_reset {
        restart_host(ctx).await?;
        wait_for_operations(ctx, outcome).await?;
    }
    Ok(awaiting_reset)
}

/// Waits until every reference in `outcome` completes.
async fn wait_for_operations(ctx: &Ctx, outcome: &DriverOutcome) -> Result<(), Outcome> {
    for reference in outcome.references() {
        poll(JOB_TIMEOUT, JOB_POLL_INTERVAL, async || {
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
    }
    Ok(())
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

fn uris(outcome: &DriverOutcome) -> String {
    outcome
        .references()
        .map(|reference| reference.uri().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
