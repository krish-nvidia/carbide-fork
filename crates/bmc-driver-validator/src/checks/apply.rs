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
//! the driver's status until the settings are in effect. A BMC can drop
//! queued BIOS work on each BIOS write, so settings checks do not share a
//! restart.
//! Power, lockdown, and account checks restore what they change.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use bmc_platform::{
    BootInterfaceSelector, Capability, ConsoleState, DriverOutcome, LockdownDesiredState,
    LockdownState, PlatformError,
};
use carbide_secrets::credentials::Credentials;
use nv_redfish::account::ManagerAccountCreate;
use nv_redfish::resource::{PowerState, ResetType};

use super::ops::{
    self, Attempt, QUICK_POLL_INTERVAL, SETTLE_POLL_INTERVAL, SETTLE_TIMEOUT, how, poll, took,
};
use super::{Check, Ctx, Needs, Outcome, Step, boot_order_summary, differences, failed, needed};
use crate::connect::Target;

/// How long a new or deleted account may take to affect logins.
const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the BMC may take to list the boot interface's boot option after
/// the BIOS settings that create it read back.
const BOOT_OPTION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// How long the settings still differing after a partial apply must read
/// the same before they are applied again.
const STALLED_FOR: Duration = Duration::from_secs(2 * 60);

pub(super) fn checks() -> Vec<Check> {
    vec![
        bios(),
        boot_order(),
        console(),
        power_cycle(),
        accounts_lifecycle(),
        lockdown_toggle(),
    ]
}

pub(super) fn bios() -> Check {
    Check::apply("bios.apply", Capability::Bios, run!(bios_apply))
}

pub(super) fn boot_order() -> Check {
    Check::apply(
        "boot_order.configure",
        Capability::BootOrder,
        run!(boot_order_configure),
    )
    .needs(Needs::BootInterface)
}

pub(super) fn console() -> Check {
    Check::apply("console.setup", Capability::Console, run!(console_setup))
}

pub(super) fn power_cycle() -> Check {
    Check::apply("power.cycle", Capability::Power, run!(cycle_power)).once()
}

pub(super) fn accounts_lifecycle() -> Check {
    Check::apply(
        "accounts.lifecycle",
        Capability::Accounts,
        run!(account_lifecycle),
    )
    .once()
}

fn lockdown_toggle() -> Check {
    Check::apply(
        "lockdown.toggle",
        Capability::Lockdown,
        run!(toggle_lockdown),
    )
    .once()
}

async fn bios_apply(ctx: &Ctx) -> Step {
    let bios = ctx.bmc.bios()?;
    let profile = &ctx.inputs.bios_profile;
    let boot_interface = ctx.inputs.boot_interface();
    let boot_interface = boot_interface.as_ref();
    apply_until_in_effect(
        ctx,
        async || {
            let status = bios
                .status(profile, boot_interface)
                .await
                .map_err(failed("status"))?;
            Ok((!status.is_applied).then(|| NotInEffect {
                summary: differences(
                    status
                        .differences
                        .iter()
                        .map(|diff| (diff.key.as_str(), &diff.expected, diff.actual.as_ref())),
                ),
                settings: status
                    .differences
                    .iter()
                    .map(|diff| diff.key.clone())
                    .collect(),
            }))
        },
        async || {
            bios.apply(profile, boot_interface)
                .await
                .map_err(failed("apply"))
        },
    )
    .await
}

async fn boot_order_configure(ctx: &Ctx) -> Step {
    let boot_order = ctx.bmc.boot_order()?;
    let mac = needed(ctx.inputs.boot_mac, Needs::BootInterface)?;
    let selector = BootInterfaceSelector::Mac(mac);
    apply_until_in_effect(
        ctx,
        async || {
            let status = boot_order
                .status(&selector)
                .await
                .map_err(failed("status"))?;
            Ok((!status.is_configured())
                .then(|| NotInEffect::described(boot_order_summary(status))))
        },
        // The BMC lists a boot option only once the host has booted with the
        // BIOS settings that create it, a while after they read back.
        async || {
            poll(
                ctx,
                BOOT_OPTION_TIMEOUT,
                SETTLE_POLL_INTERVAL,
                async || match boot_order.configure(&selector).await {
                    Ok(outcome) => Attempt::Done(outcome),
                    Err(PlatformError::MissingBootOption { description }) => {
                        Attempt::NotYet(format!("the BMC does not list {description} yet"))
                    }
                    Err(error) => Attempt::Failed(failed("configure")(error)),
                },
            )
            .await
        },
    )
    .await
}

async fn console_setup(ctx: &Ctx) -> Step {
    let console = ctx.bmc.console()?;
    apply_until_in_effect(
        ctx,
        async || {
            let status = console.status().await.map_err(failed("status"))?;
            Ok((status.state != ConsoleState::Enabled).then(|| {
                NotInEffect::described(format!("{:?} ({})", status.state, status.message))
            }))
        },
        async || console.setup().await.map_err(failed("setup")),
    )
    .await
}

/// Why settings are not yet in effect: a summary, and the settings that
/// differ when the status names them.
struct NotInEffect {
    summary: String,
    settings: BTreeSet<String>,
}

impl NotInEffect {
    /// A reason that names no individual settings.
    fn described(summary: String) -> Self {
        Self {
            summary,
            settings: BTreeSet::new(),
        }
    }
}

/// Applies settings and verifies they take effect: `pending` reads why the
/// settings are not yet in effect, or `None` once they are. When the first
/// restart applies only some of the named settings, they are applied and
/// the host restarted once more. Settings already in effect are applied once
/// more, which must start no work.
async fn apply_until_in_effect(
    ctx: &Ctx,
    pending: impl AsyncFn() -> Result<Option<NotInEffect>, Outcome>,
    apply: impl AsyncFn() -> Result<DriverOutcome, Outcome>,
) -> Step {
    let Some(before) = pending().await? else {
        ctx.trace("already in effect; applying again, which must start no work");
        return reapply_changes_nothing(ctx, &apply, "already in effect").await;
    };
    ctx.trace(format!("not in effect: {}", before.summary));
    let started = Instant::now();
    let outcome = apply().await?;
    ctx.trace(format!("apply: {}", how(&outcome)));
    ops::take_effect(ctx, &outcome).await?;
    let mut applied = format!("applying ({}) and a host restart", how(&outcome));
    if let Some(rest) = wait_in_effect(ctx, &pending, Some(&before.settings)).await? {
        let took_effect = before
            .settings
            .iter()
            .filter(|setting| !rest.settings.contains(*setting))
            .count();
        ctx.trace(format!(
            "{took_effect} of {} settings took effect, but {} still differ; applying again, \
             as a setting can depend on another applying first",
            before.settings.len(),
            rest.summary
        ));
        let again = apply().await?;
        ctx.trace(format!("apply again: {}", how(&again)));
        ops::take_effect(ctx, &again).await?;
        wait_in_effect(ctx, &pending, None).await?;
        applied.push_str(&format!(
            ", then applying again ({}) and a second restart",
            how(&again)
        ));
    }
    ctx.trace("in effect; applying again, which must start no work");
    let state = format!("in effect {} after {applied}", took(started));
    reapply_changes_nothing(ctx, &apply, &state).await
}

/// Whether some settings that differed `before` took effect while others
/// still differ, which can include settings the BIOS reports only now.
fn partly_in_effect(before: &BTreeSet<String>, differing: &BTreeSet<String>) -> bool {
    !differing.is_empty() && before.iter().any(|setting| !differing.contains(setting))
}

/// Polls `pending` until the settings are in effect, returning `None`.
/// Given the settings that differed `before` applying, it instead returns
/// what still differs once some of them took effect and what differs has
/// read the same for [`STALLED_FOR`]: a BIOS applies, or even reports, a
/// setting that depends on another, such as a TPM device behind TPM
/// support, only on the boot after that one applies.
async fn wait_in_effect(
    ctx: &Ctx,
    pending: &impl AsyncFn() -> Result<Option<NotInEffect>, Outcome>,
    before: Option<&BTreeSet<String>>,
) -> Result<Option<NotInEffect>, Outcome> {
    let mut stalled: Option<(BTreeSet<String>, Instant)> = None;
    poll(
        ctx,
        SETTLE_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match pending().await {
            Ok(None) => Attempt::Done(None),
            Ok(Some(rest)) => {
                let partial = before.is_some_and(|before| partly_in_effect(before, &rest.settings));
                if !partial {
                    stalled = None;
                } else if let Some((settings, since)) = &stalled
                    && *settings == rest.settings
                {
                    if since.elapsed() >= STALLED_FOR {
                        return Attempt::Done(Some(rest));
                    }
                } else {
                    stalled = Some((rest.settings.clone(), Instant::now()));
                }
                Attempt::NotYet(format!("not in effect: {}", rest.summary))
            }
            // The BMC can refuse BIOS reads while the host POSTs.
            Err(Outcome::Fail(reason) | Outcome::Unreachable(reason)) => Attempt::NotYet(reason),
            Err(other) => Attempt::Failed(other),
        },
    )
    .await
}

async fn reapply_changes_nothing(
    ctx: &Ctx,
    apply: &impl AsyncFn() -> Result<DriverOutcome, Outcome>,
    state: &str,
) -> Step {
    match apply().await? {
        DriverOutcome::Complete => Ok(format!("{state}; applying again changed nothing")),
        accepted => {
            let uris = ops::uris(&accepted);
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
async fn cycle_power(ctx: &Ctx) -> Step {
    let Some(original) = ops::power_state(&ctx.bmc).await.map_err(failed("state"))? else {
        return Err(Outcome::Skipped(
            "the platform reports no single power state".to_string(),
        ));
    };
    let (away, back) = match original {
        PowerState::Off => (ResetType::On, ResetType::ForceOff),
        _ => (ResetType::ForceOff, ResetType::On),
    };
    let away_result = ops::set_power(ctx, &ctx.bmc, away).await;
    let back_result = ops::set_power(ctx, &ctx.bmc, back).await;
    match ops::power_state(&ctx.bmc).await {
        Ok(Some(state)) if state == original => {}
        current => ctx.not_restored(format!(
            "host power was {original:?}; it now reads {current:?}"
        )),
    }
    Ok(format!("{}; {}", away_result?, back_result?))
}

/// Creates a temporary account, logs in with it, deletes it, and confirms
/// the login stops working; the run's own account is never changed.
async fn account_lifecycle(ctx: &Ctx) -> Step {
    let accounts = ctx.bmc.accounts()?;
    let username = format!("nicoval{:04}", rand::random::<u16>() % 10_000);
    let password = Credentials::generate_password();
    ctx.redactor.add(&password);
    let temporary = ctx.target.as_account(&username, &password);

    if listed(ctx, &username).await? {
        return Err(Outcome::Skipped(format!(
            "an account named {username} already exists"
        )));
    }
    let created = accounts
        .create(ManagerAccountCreate::builder(
            password.clone(),
            username.clone(),
            "Administrator".to_string(),
        ))
        .await;
    match &created {
        Ok(outcome) => ctx.trace(format!("create {username}: {}", how(outcome))),
        Err(error) => ctx.trace(format!("create {username}: {error}")),
    }
    let verified = match created {
        Ok(outcome) => verify_new_account(ctx, &outcome, &username, &temporary).await,
        Err(error) => Err(failed("create")(error)),
    };
    if !listed(ctx, &username).await.unwrap_or(true) {
        return verified;
    }

    let deleted = accounts.delete(&username).await.map_err(failed("delete"));
    if let Ok(outcome) = &deleted {
        ctx.trace(format!("delete {username}: {}", how(outcome)));
    }
    let gone = match deleted {
        Ok(outcome) => verify_account_gone(ctx, &outcome, &username, &temporary).await,
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
    temporary: &Target,
) -> Step {
    ops::wait_for_operations(ctx, outcome).await?;
    if !listed(ctx, username).await? {
        return Err(Outcome::Fail(format!(
            "created {username}, but it is not listed"
        )));
    }
    poll(
        ctx,
        ACCOUNT_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match temporary.log_in().await {
            Ok(()) => Attempt::Done(()),
            Err(error) => Attempt::NotYet(format!("logging in as {username} fails: {error}")),
        },
    )
    .await?;
    ctx.trace(format!("logged in as {username}"));
    Ok(format!("created {username} and logged in with it"))
}

async fn verify_account_gone(
    ctx: &Ctx,
    outcome: &DriverOutcome,
    username: &str,
    temporary: &Target,
) -> Step {
    ops::wait_for_operations(ctx, outcome).await?;
    if listed(ctx, username).await? {
        return Err(Outcome::Fail(format!(
            "deleted {username}, but it is still listed"
        )));
    }
    poll(
        ctx,
        ACCOUNT_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match temporary.log_in().await {
            Err(PlatformError::Auth(_)) => Attempt::Done(()),
            Err(error) => Attempt::NotYet(format!(
                "logging in as {username} fails with {error}, not an auth error"
            )),
            Ok(()) => Attempt::NotYet(format!("{username} can still log in")),
        },
    )
    .await?;
    Ok("deleted it, and its login is refused".to_string())
}

async fn listed(ctx: &Ctx, username: &str) -> Result<bool, Outcome> {
    let accounts = ctx.bmc.accounts()?.list().await.map_err(failed("list"))?;
    Ok(accounts
        .iter()
        .any(|account| account.user_name.as_deref() == Some(username)))
}

/// Locks down a host that is not locked down, or unlocks one that is, then
/// restores it.
async fn toggle_lockdown(ctx: &Ctx) -> Step {
    let lockdown = ctx.bmc.lockdown()?;
    let original = lockdown.status().await.map_err(failed("status"))?.aggregate;
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
    let away_result = ops::set_lockdown(ctx, away).await;
    let back_result = ops::set_lockdown(ctx, back).await;
    match lockdown.status().await {
        Ok(status) if status.aggregate == original => {}
        current => ctx.not_restored(format!(
            "lockdown was {original:?}; it now reads {:?}",
            current.map(|status| status.aggregate)
        )),
    }
    Ok(format!("{}; {}", away_result?, back_result?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second apply is worth a restart only once the first applied some
    /// settings; settings the BIOS reports only after it count as differing.
    #[test]
    fn a_partial_apply_is_some_settings_taking_effect() {
        let set = |names: &[&str]| {
            names
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
        };
        let before = set(&["QuietBoot", "SecurityDeviceSupport", "IPv4PXESupport"]);
        let cases = [
            (
                "nothing took effect yet",
                set(&["QuietBoot", "SecurityDeviceSupport", "IPv4PXESupport"]),
                false,
            ),
            ("some took effect", set(&["IPv4PXESupport"]), true),
            (
                "some took effect and a hidden setting appeared",
                set(&["DeviceSelect_0034", "DeviceSelect_003D"]),
                true,
            ),
            ("everything took effect", set(&[]), false),
        ];
        for (scenario, differing, partial) in cases {
            assert_eq!(partly_in_effect(&before, &differing), partial, "{scenario}");
        }
    }
}
