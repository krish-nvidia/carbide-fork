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

//! The `--validate` sequence: every capability the selection supports, in
//! the order NICo takes a BMC from factory state to ready, with a restart
//! wherever a change takes effect on one.

use std::time::{Duration, Instant, SystemTime};

use bmc_platform::{
    AuthError, Capability, HostPrivilegeLevel, LockdownDesiredState, ManagerSettings, NicMode,
    OpCx, PlatformError, RshimState, SecureBootStatus, TimeStatus,
};
use bmc_runtime::ConnectedBmc;
use carbide_redfish::nv_redfish::RedfishBmc;
use nv_redfish::account::AccountServiceConfig;
use nv_redfish::resource::ResetType;
use nv_redfish::schema::computer_system::{BootSource, BootSourceOverrideEnabled, BootUpdate};
use nv_redfish::schema::secure_boot::SecureBootUpdate;

use super::ops::{
    self, Attempt, BMC_UP_TIMEOUT, IMMEDIATE_TIMEOUT, QUICK_POLL_INTERVAL, SETTLE_POLL_INTERVAL,
    SETTLE_TIMEOUT, how, poll, took,
};
use super::{
    Check, Checked, Ctx, Detail, Needs, Outcome, Step, apply, describe, difference_detail, failed,
    needed, reads, reported, unreadable,
};
use crate::connect::Target;
use crate::selection::Selection;

/// What `--validate` leaves the BMC in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum EndState {
    /// Configured and locked down, as NICo leaves it.
    #[default]
    NicoReady,
    /// Reset to factory defaults, with the factory password.
    Factory,
}

/// How far the BMC's clock may be from this machine's once NTP syncs it.
const CLOCK_TOLERANCE: Duration = Duration::from_secs(5 * 60);
const CLOCK_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How long a factory reset may take before the factory password works.
const FACTORY_RESET_TIMEOUT: Duration = Duration::from_secs(20 * 60);

const BMC_RESET: &str = "bmc_control.reset";

pub(super) fn checks(dpu: bool, end_state: EndState) -> Vec<Check> {
    let mut checks = vec![
        Check::apply(
            "accounts.policy",
            Capability::Accounts,
            run!(account_policy),
        ),
        apply::accounts_lifecycle(),
        Check::apply(BMC_RESET, Capability::BmcControl, run!(ops::reset_bmc)).once(),
        Check::apply("bmc_control.ntp", Capability::BmcControl, run!(ntp))
            .needs(Needs::NtpServers)
            .after(&[BMC_RESET]),
        Check::apply("bmc_control.utc", Capability::BmcControl, run!(utc)).after(&[BMC_RESET]),
    ];
    checks.extend(if dpu { dpu_setup() } else { host_setup() });
    checks.extend(reads::checks());
    if end_state == EndState::Factory {
        checks.extend(factory_reset());
    }
    checks
}

/// The machine controller's host initialization, lockdown last.
fn host_setup() -> Vec<Check> {
    vec![
        Check::apply(
            "bmc_control.enable_ipmi_over_lan",
            Capability::BmcControl,
            run!(ipmi_over_lan),
        )
        .after(&[BMC_RESET]),
        Check::apply("lockdown.lift", Capability::Lockdown, run!(lift_lockdown))
            .after(&["bmc_control.enable_ipmi_over_lan"]),
        Check::apply(
            "bmc_control.settings",
            Capability::BmcControl,
            run!(manager_settings),
        )
        .after(&["lockdown.lift"]),
        apply::bios().after(&["lockdown.lift"]),
        apply::boot_order().after(&["bios.apply"]),
        apply::console().after(&["bios.apply"]),
        apply::power_cycle().after(&["console.setup"]),
        Check::apply("power.ac_cycle", Capability::Power, run!(ac_cycle))
            .once()
            .needs(Needs::AcCycle)
            .after(&["power.cycle"]),
        uefi_password().after(&["bios.apply"]),
        Check::apply(
            "lockdown.enable",
            Capability::Lockdown,
            run!(enable_lockdown),
        )
        .after(&[
            "lockdown.lift",
            "bmc_control.settings",
            "bios.apply",
            "boot_order.configure",
            "console.setup",
            "power.cycle",
            "power.ac_cycle",
            "bios.uefi_password",
        ]),
    ]
}

/// DPU discovery and initialization, with the DPU's own restarts.
fn dpu_setup() -> Vec<Check> {
    vec![
        Check::apply("dpu.bmc_rshim", Capability::Dpu, run!(bmc_rshim)).after(&[BMC_RESET]),
        Check::apply("dpu.nic_mode", Capability::Dpu, run!(nic_mode))
            .needs(Needs::HostBmc)
            .after(&[BMC_RESET]),
        Check::apply("dpu.host_privilege", Capability::Dpu, run!(host_privilege))
            .after(&["dpu.nic_mode"]),
        apply::bios().after(&["dpu.host_privilege"]),
        Check::apply(
            "secure_boot.enable",
            Capability::SecureBoot,
            run!(secure_boot),
        )
        .after(&["bios.apply"]),
        Check::apply(
            "boot_order.http_boot_once",
            Capability::BootOrder,
            run!(http_boot_once),
        )
        .after(&["secure_boot.enable"]),
        Check::apply("dpu.host_rshim", Capability::Dpu, run!(host_rshim)).after(&["dpu.nic_mode"]),
        apply::console(),
        uefi_password().after(&["bios.apply"]),
    ]
}

/// Back to factory state; each step runs whatever came before.
fn factory_reset() -> Vec<Check> {
    vec![
        Check::apply(
            "factory.lockdown",
            Capability::Lockdown,
            run!(lift_lockdown),
        ),
        Check::apply(
            "factory.uefi_password",
            Capability::Bios,
            run!(clear_uefi_password),
        )
        .once()
        .needs(Needs::UefiPassword),
        Check::apply("factory.bios", Capability::Bios, run!(reset_bios)),
        Check::apply(
            "factory.bmc",
            Capability::BmcControl,
            run!(reset_to_factory),
        )
        .once(),
    ]
}

fn uefi_password() -> Check {
    Check::apply(
        "bios.uefi_password",
        Capability::Bios,
        run!(set_uefi_password),
    )
    .once()
    .needs(Needs::UefiPassword)
}

async fn account_policy(ctx: &Ctx) -> Checked {
    let outcome = ctx
        .bmc
        .accounts()?
        .apply_default_policy()
        .await
        .map_err(failed("apply_default_policy"))?;
    ctx.trace(format!("apply_default_policy: {}", how(&outcome)));
    ops::wait_for_operations(ctx, &outcome).await?;
    let service = async {
        ctx.target
            .fresh_connection()
            .await?
            .service_root
            .account_service(AccountServiceConfig::standard())
            .await
            .map_err(PlatformError::from_redfish)?
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: "the BMC lists no AccountService".to_string(),
            })
    }
    .await
    .map_err(unreadable("AccountService"))?;
    let raw = service.raw();
    let fields = vec![
        (
            "lockout threshold".to_string(),
            reported(&raw.account_lockout_threshold.flatten()),
        ),
        (
            "lockout duration".to_string(),
            reported(&raw.account_lockout_duration.flatten()),
        ),
        (
            "counter reset after".to_string(),
            reported(&raw.account_lockout_counter_reset_after),
        ),
    ];
    Ok(Detail {
        summary: format!("applied ({}); AccountService now reads", how(&outcome)),
        fields,
    })
}

/// Sets the NTP servers and waits until they are listed and the clock is
/// within [`CLOCK_TOLERANCE`] of this machine's.
async fn ntp(ctx: &Ctx) -> Step {
    let servers = &ctx.inputs.ntp_servers;
    let outcome = ctx
        .bmc
        .bmc_control()?
        .set_ntp_servers(servers)
        .await
        .map_err(failed("set_ntp_servers"))?;
    ctx.trace(format!("set_ntp_servers: {}", how(&outcome)));
    ops::wait_for_operations(ctx, &outcome).await?;
    let control = ctx.bmc.bmc_control()?;
    let listed = poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match control.time_status().await {
            Ok(status) if lists_ntp_servers(&status, servers) => Attempt::Done(status.ntp_servers),
            Ok(status) => Attempt::NotYet(format!(
                "NTP is enabled: {}; it lists {}",
                reported(&status.ntp_enabled),
                status.ntp_servers.join(", ")
            )),
            Err(error) => Attempt::NotYet(describe("time_status", &error)),
        },
    )
    .await?;
    let skew = poll(
        ctx,
        CLOCK_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match control.time_status().await.map(|status| status.now) {
            Ok(None) => Attempt::Done(None),
            Ok(Some(now)) => {
                let skew = now
                    .duration_since(SystemTime::now())
                    .unwrap_or_else(|behind| behind.duration());
                if skew <= CLOCK_TOLERANCE {
                    Attempt::Done(Some(skew))
                } else {
                    Attempt::NotYet(format!("the BMC clock is {}s off", skew.as_secs()))
                }
            }
            Err(error) => Attempt::NotYet(describe("time_status", &error)),
        },
    )
    .await?;
    Ok(format!(
        "lists {} ({}); {}",
        listed.join(", "),
        how(&outcome),
        match skew {
            Some(skew) => format!("the clock is within {}s of this machine's", skew.as_secs()),
            None => "the BMC reports no clock".to_string(),
        }
    ))
}

/// Whether the BMC lists the servers in `wanted` it has room for: as many as
/// it lists entries, from the front, with NTP not switched off.
fn lists_ntp_servers(status: &TimeStatus, wanted: &[String]) -> bool {
    let kept = status.ntp_servers.len().min(wanted.len());
    status.ntp_enabled != Some(false)
        && kept > 0
        && wanted[..kept]
            .iter()
            .all(|server| status.ntp_servers.contains(server))
}

async fn utc(ctx: &Ctx) -> Step {
    const UTC: &[&str] = &["+00:00", "-00:00", "Z"];
    let control = ctx.bmc.bmc_control()?;
    let outcome = control
        .set_utc_timezone()
        .await
        .map_err(failed("set_utc_timezone"))?;
    ctx.trace(format!("set_utc_timezone: {}", how(&outcome)));
    ops::wait_for_operations(ctx, &outcome).await?;
    let offset = poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match control.time_status().await.map(|status| status.utc_offset) {
            Ok(None) => Attempt::Done(None),
            Ok(Some(offset)) if UTC.contains(&offset.as_str()) => Attempt::Done(Some(offset)),
            Ok(Some(offset)) => Attempt::NotYet(format!("the UTC offset is {offset}")),
            Err(error) => Attempt::NotYet(describe("time_status", &error)),
        },
    )
    .await?;
    Ok(format!(
        "{} ({})",
        match offset {
            Some(offset) => format!("the UTC offset is {offset}"),
            None => "the BMC reports no UTC offset".to_string(),
        },
        how(&outcome)
    ))
}

async fn ipmi_over_lan(ctx: &Ctx) -> Step {
    let control = ctx.bmc.bmc_control()?;
    let enabled = control
        .ipmi_over_lan_enabled()
        .await
        .map_err(failed("ipmi_over_lan_enabled"))?;
    let change = if enabled {
        "already enabled".to_string()
    } else {
        let outcome = control
            .set_ipmi_over_lan(true)
            .await
            .map_err(failed("set_ipmi_over_lan"))?;
        ctx.trace(format!("set_ipmi_over_lan: {}", how(&outcome)));
        ops::wait_for_operations(ctx, &outcome).await?;
        format!("enabled ({})", how(&outcome))
    };
    poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match control.ipmi_over_lan_enabled().await {
            Ok(true) => Attempt::Done(()),
            Ok(false) => Attempt::NotYet("IPMI over LAN reads disabled".to_string()),
            Err(error) => Attempt::NotYet(describe("ipmi_over_lan_enabled", &error)),
        },
    )
    .await?;
    Ok(format!("{change}; IPMI over LAN reads enabled"))
}

async fn lift_lockdown(ctx: &Ctx) -> Step {
    ops::set_lockdown(ctx, LockdownDesiredState::Disabled).await
}

async fn enable_lockdown(ctx: &Ctx) -> Step {
    ops::set_lockdown(ctx, LockdownDesiredState::Enabled).await
}

async fn manager_settings(ctx: &Ctx) -> Checked {
    let control = ctx.bmc.bmc_control()?;
    let outcome = control
        .apply_settings(&ManagerSettings::default())
        .await
        .map_err(failed("apply_settings"))?;
    ctx.trace(format!("apply_settings: {}", how(&outcome)));
    ops::wait_for_operations(ctx, &outcome).await?;
    let status = poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match control.settings_status().await {
            Ok(status) if status.is_applied => Attempt::Done(status),
            Ok(status) => Attempt::NotYet(format!("{} settings differ", status.differences.len())),
            Err(error) => Attempt::NotYet(describe("settings_status", &error)),
        },
    )
    .await?;
    let detail = difference_detail(
        status
            .differences
            .iter()
            .map(|diff| (diff.key.as_str(), &diff.expected, diff.actual.as_ref())),
    );
    Ok(Detail {
        summary: format!("{} ({})", detail.summary, how(&outcome)),
        ..detail
    })
}

/// Requests an AC power cycle and waits for the BMC to lose power with the
/// host, then for it to answer again, for the work the request started, and
/// for the host to be on. A BMC accepts the request before cutting power, so
/// the cycle is only taken as done once the BMC drops.
async fn ac_cycle(ctx: &Ctx) -> Step {
    let power = ctx.bmc.power()?;
    if !power
        .ac_power_cycle_supported()
        .await
        .map_err(failed("ac_power_cycle_supported"))?
    {
        return Err(Outcome::Unsupported(
            "the driver reports no AC power cycle for this platform".to_string(),
        ));
    }
    let started = Instant::now();
    let outcome = ops::sent_or_dropped(ctx, "set", power.set(ResetType::FullPowerCycle).await)?;
    ctx.trace(format!("power FullPowerCycle: {}", how(&outcome)));
    ops::wait_for_bmc_down(ctx, ops::ac_cycle_down_timeout(&outcome))
        .await
        .map_err(|outcome| match outcome {
            Outcome::Fail(reason) => {
                Outcome::Fail(format!("no power cycle was observed: {reason}"))
            }
            other => other,
        })?;
    ops::wait_for_login(ctx, &ctx.target, BMC_UP_TIMEOUT).await?;
    ops::settle(ctx, &outcome).await?;
    ops::power_on(ctx).await?;
    Ok(format!(
        "the BMC lost power, and everything was back on {} after the request ({})",
        took(started),
        how(&outcome)
    ))
}

/// Sets the UEFI password and restarts the host so it applies. The password
/// cannot be read back, so an accepted request is taken as set.
async fn set_uefi_password(ctx: &Ctx) -> Step {
    let password = needed(ctx.inputs.uefi_password.as_ref(), Needs::UefiPassword)?;
    let outcome = ctx
        .bmc
        .bios()?
        .change_uefi_password(&password.current, &password.new)
        .await
        .map_err(failed("change_uefi_password"))?;
    ops::take_effect(ctx, &outcome).await?;
    Ok(format!("set ({}) and restarted", how(&outcome)))
}

async fn clear_uefi_password(ctx: &Ctx) -> Step {
    let password = needed(ctx.inputs.uefi_password.as_ref(), Needs::UefiPassword)?;
    let outcome = ctx
        .bmc
        .bios()?
        .clear_uefi_password(&password.new)
        .await
        .map_err(failed("clear_uefi_password"))?;
    ops::take_effect(ctx, &outcome).await?;
    Ok(format!("cleared ({}) and restarted", how(&outcome)))
}

async fn reset_bios(ctx: &Ctx) -> Step {
    let outcome = ctx.bmc.bios()?.reset().await.map_err(failed("reset"))?;
    ops::take_effect(ctx, &outcome).await?;
    Ok(format!(
        "reset to defaults ({}) and restarted",
        how(&outcome)
    ))
}

/// Resets the BMC to factory defaults and waits for the factory password.
async fn reset_to_factory(ctx: &Ctx) -> Step {
    let factory = ctx.inputs.factory.as_ref().ok_or_else(|| {
        Outcome::Skipped("the factory credentials are only known to --validate".to_string())
    })?;
    let started = Instant::now();
    let outcome = ctx
        .bmc
        .bmc_control()?
        .reset_to_factory_defaults()
        .await
        .map_err(failed("reset_to_factory_defaults"))?;
    ctx.trace(format!("reset_to_factory_defaults: {}", how(&outcome)));
    poll(
        ctx,
        FACTORY_RESET_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match factory.log_in().await {
            Ok(()) | Err(PlatformError::Auth(AuthError::PasswordChangeRequired { .. })) => {
                Attempt::Done(())
            }
            Err(error) => Attempt::NotYet(format!("the factory password fails: {error}")),
        },
    )
    .await?;
    Ok(format!(
        "the factory password works again after {} ({})",
        took(started),
        how(&outcome)
    ))
}

async fn bmc_rshim(ctx: &Ctx) -> Step {
    let dpu = ctx.bmc.dpu()?;
    let outcome = dpu
        .enable_bmc_rshim()
        .await
        .map_err(failed("enable_bmc_rshim"))?;
    ops::wait_for_operations(ctx, &outcome).await?;
    poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match dpu.status().await {
            Ok(status) if status.bmc_rshim == Some(RshimState::Enabled) => Attempt::Done(()),
            Ok(status) => {
                Attempt::NotYet(format!("BMC rshim reads {}", reported(&status.bmc_rshim)))
            }
            Err(error) => Attempt::NotYet(describe("status", &error)),
        },
    )
    .await?;
    Ok(format!("BMC rshim reads Enabled ({})", how(&outcome)))
}

/// Switches the DPU to DPU mode, which takes a host power cycle.
async fn nic_mode(ctx: &Ctx) -> Step {
    let dpu = ctx.bmc.dpu()?;
    let status = dpu.status().await.map_err(failed("status"))?;
    if status.nic_mode == Some(NicMode::Dpu) {
        return Ok("already in DPU mode".to_string());
    }
    let host = needed(ctx.inputs.host_bmc.as_ref(), Needs::HostBmc)?;
    let started = Instant::now();
    let outcome = dpu
        .set_nic_mode(NicMode::Dpu)
        .await
        .map_err(failed("set_nic_mode"))?;
    ctx.trace(format!("set_nic_mode: {}", how(&outcome)));
    ops::settle(ctx, &outcome).await?;
    let host = connect_host(host).await?;
    ctx.trace("power-cycling the host through its BMC");
    ops::set_power(ctx, &host, ResetType::ForceOff).await?;
    ops::set_power(ctx, &host, ResetType::On).await?;
    ops::wait_for_login(ctx, &ctx.target, BMC_UP_TIMEOUT).await?;
    poll(
        ctx,
        SETTLE_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match dpu.status().await {
            Ok(status) if status.nic_mode == Some(NicMode::Dpu) => Attempt::Done(()),
            Ok(status) => Attempt::NotYet(format!(
                "the DPU reports nic mode {}",
                reported(&status.nic_mode)
            )),
            Err(error) => Attempt::NotYet(describe("status", &error)),
        },
    )
    .await?;
    Ok(format!(
        "DPU mode {} after a host power cycle ({})",
        took(started),
        how(&outcome)
    ))
}

/// Discovers the host BMC and connects to it with the drivers its own
/// identity selects.
async fn connect_host(host: &Target) -> Result<ConnectedBmc<RedfishBmc>, Outcome> {
    let reach = async {
        let identity = host.discover().await?;
        let selection = Selection::resolve(&identity, &[])?;
        host.connect(selection.endpoint(host.address(), identity)?)
            .await
    };
    reach
        .await
        .map_err(|error| Outcome::Fail(format!("host BMC {}: {error}", host.address())))
}

async fn host_privilege(ctx: &Ctx) -> Step {
    let dpu = ctx.bmc.dpu()?;
    let outcome = dpu
        .set_host_privilege_level(HostPrivilegeLevel::Restricted)
        .await
        .map_err(failed("set_host_privilege_level"))?;
    ctx.trace(format!("set_host_privilege_level: {}", how(&outcome)));
    ops::take_effect(ctx, &outcome).await?;
    poll(
        ctx,
        SETTLE_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match dpu.status().await {
            Ok(status) if status.host_privilege == Some(HostPrivilegeLevel::Restricted) => {
                Attempt::Done(())
            }
            Ok(status) => Attempt::NotYet(format!(
                "host privilege reads {}",
                reported(&status.host_privilege)
            )),
            Err(error) => Attempt::NotYet(describe("status", &error)),
        },
    )
    .await?;
    Ok(format!(
        "host privilege reads Restricted after a DPU restart ({})",
        how(&outcome)
    ))
}

async fn secure_boot(ctx: &Ctx) -> Step {
    let secure_boot = ctx.bmc.secure_boot()?;
    let mut outcome = secure_boot
        .set(
            &SecureBootUpdate::builder()
                .with_secure_boot_enable(true)
                .build(),
        )
        .await
        .map_err(failed("set"))?;
    if let Some(key) = &ctx.inputs.platform_key {
        outcome = outcome.merge(
            secure_boot
                .add_platform_key(key)
                .await
                .map_err(failed("add_platform_key"))?,
        );
    }
    ctx.trace(format!("enable secure boot: {}", how(&outcome)));
    ops::take_effect(ctx, &outcome).await?;
    let want_key = ctx.inputs.platform_key.is_some();
    poll(ctx, SETTLE_TIMEOUT, SETTLE_POLL_INTERVAL, async || {
        let status = secure_boot.status().await;
        let key = secure_boot.has_platform_key().await;
        match (status, key) {
            (Ok(SecureBootStatus::Enabled), Ok(key)) if key || !want_key => Attempt::Done(()),
            (Ok(status), Ok(key)) => Attempt::NotYet(format!(
                "secure boot reads {status:?}; platform key enrolled: {key}"
            )),
            (Err(error), _) | (_, Err(error)) => {
                Attempt::NotYet(describe("secure boot status", &error))
            }
        }
    })
    .await?;
    Ok(format!(
        "Enabled{} after a DPU restart ({})",
        if want_key {
            ", platform key enrolled"
        } else {
            ""
        },
        how(&outcome)
    ))
}

/// The system's boot override as `(target, enabled)`, read over a fresh
/// connection from the system the drivers act on.
async fn boot_override(
    ctx: &Ctx,
) -> Result<(Option<BootSource>, Option<BootSourceOverrideEnabled>), PlatformError> {
    let connection = ctx.target.fresh_connection().await?;
    let cx = OpCx::new(
        connection.bmc.as_ref(),
        connection.service_root.as_ref(),
        ctx.bmc.endpoint().identity(),
    );
    let raw = cx.system().await?.raw();
    let boot = raw.boot.as_ref();
    Ok((
        boot.and_then(|boot| boot.boot_source_override_target.flatten()),
        boot.and_then(|boot| boot.boot_source_override_enabled.flatten()),
    ))
}

/// Sets a one-time UEFI HTTP boot, restarts the DPU, and checks the
/// override was consumed.
async fn http_boot_once(ctx: &Ctx) -> Step {
    let http_once = (
        Some(BootSource::UefiHttp),
        Some(BootSourceOverrideEnabled::Once),
    );
    let outcome = ctx
        .bmc
        .boot_order()?
        .set_override(
            &BootUpdate::builder()
                .with_boot_source_override_target(BootSource::UefiHttp)
                .with_boot_source_override_enabled(BootSourceOverrideEnabled::Once)
                .build(),
        )
        .await
        .map_err(failed("set_override"))?;
    ctx.trace(format!("set_override: {}", how(&outcome)));
    ops::wait_for_operations(ctx, &outcome).await?;
    poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match boot_override(ctx).await {
            Ok(set) if set == http_once => Attempt::Done(()),
            Ok((target, enabled)) => Attempt::NotYet(format!(
                "the override reads {} {}",
                reported(&target),
                reported(&enabled)
            )),
            Err(error) => Attempt::NotYet(describe("reading ComputerSystem.Boot", &error)),
        },
    )
    .await?;
    ops::restart_host(ctx).await?;
    poll(
        ctx,
        SETTLE_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match boot_override(ctx).await {
            Ok(set) if set == http_once => {
                Attempt::NotYet("the one-time override is still set".to_string())
            }
            Ok(_) => Attempt::Done(()),
            Err(error) => Attempt::NotYet(describe("reading ComputerSystem.Boot", &error)),
        },
    )
    .await?;
    Ok(format!(
        "UefiHttp once was set ({}) and consumed by a DPU restart",
        how(&outcome)
    ))
}

async fn host_rshim(ctx: &Ctx) -> Step {
    let dpu = ctx.bmc.dpu()?;
    let outcome = dpu
        .set_host_rshim(RshimState::Enabled)
        .await
        .map_err(failed("set_host_rshim"))?;
    ops::wait_for_operations(ctx, &outcome).await?;
    poll(
        ctx,
        IMMEDIATE_TIMEOUT,
        QUICK_POLL_INTERVAL,
        async || match dpu.status().await {
            Ok(status) if status.host_rshim == Some(RshimState::Enabled) => Attempt::Done(()),
            Ok(status) => {
                Attempt::NotYet(format!("host rshim reads {}", reported(&status.host_rshim)))
            }
            Err(error) => Attempt::NotYet(describe("status", &error)),
        },
    )
    .await?;
    Ok(format!("host rshim Enabled ({})", how(&outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMC with fewer slots than servers given keeps the first ones; one
    /// still listing its placeholders has taken none.
    #[test]
    fn ntp_servers_are_listed_up_to_the_slots_the_bmc_has() {
        let list = |entries: &[&str]| entries.iter().map(ToString::to_string).collect::<Vec<_>>();
        let wanted = list(&["10.0.0.1", "10.0.0.2", "10.0.0.3"]);
        let cases = [
            (
                "every server listed",
                list(&["10.0.0.1", "10.0.0.2", "10.0.0.3"]),
                Some(true),
                true,
            ),
            (
                "two slots, first two kept",
                list(&["10.0.0.1", "10.0.0.2"]),
                None,
                true,
            ),
            (
                "factory placeholders",
                list(&["localhost", "127.0.0.1"]),
                Some(false),
                false,
            ),
            (
                "listed but disabled",
                list(&["10.0.0.1", "10.0.0.2"]),
                Some(false),
                false,
            ),
            ("nothing listed", list(&[]), Some(true), false),
        ];
        for (scenario, ntp_servers, ntp_enabled, listed) in cases {
            let status = TimeStatus {
                ntp_enabled,
                ntp_servers,
                ..TimeStatus::default()
            };
            assert_eq!(lists_ntp_servers(&status, &wanted), listed, "{scenario}");
        }
    }

    /// Gating looks dependencies up by id: one on a later, absent, or
    /// repeated step would gate nothing or the wrong step.
    #[test]
    fn every_dependency_names_one_earlier_step() {
        for dpu in [false, true] {
            let checks = checks(dpu, EndState::Factory);
            for (index, check) in checks.iter().enumerate() {
                for dependency in check.after {
                    let runs = |checks: &[Check]| {
                        checks
                            .iter()
                            .filter(|other| other.id == *dependency)
                            .count()
                    };
                    assert!(
                        runs(&checks[..index]) == 1 && runs(&checks) == 1,
                        "{} (dpu: {dpu}) follows {dependency}, which must run once, before it",
                        check.id
                    );
                }
            }
        }
    }
}
