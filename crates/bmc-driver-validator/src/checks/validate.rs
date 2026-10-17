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

use super::apply::{self, BIOS, BOOT_ORDER, CONSOLE, POWER_CYCLE};
use super::context::CheckCx;
use super::errors::{describe, failed};
use super::model::{Check, CheckId, Checked, Detail, Needs, Outcome, Step};
use super::ops::{
    self, Attempt, Effect, SETTLE_POLL_INTERVAL, SETTLE_TIMEOUT, how, poll, requested, took,
};
use super::reads;
use super::text::{counted, difference_detail, reported};
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

/// How long a BMC may keep answering after accepting a factory reset, and
/// then take before the factory password works.
const FACTORY_DOWN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const FACTORY_RESET_TIMEOUT: Duration = Duration::from_secs(20 * 60);

const ACCOUNT_POLICY: CheckId = CheckId::new("accounts.policy");
const BMC_RESET: CheckId = CheckId::new("bmc_control.reset");
const NTP: CheckId = CheckId::new("bmc_control.ntp");
const UTC: CheckId = CheckId::new("bmc_control.utc");
const IPMI_OVER_LAN: CheckId = CheckId::new("bmc_control.enable_ipmi_over_lan");
const LOCKDOWN_LIFT: CheckId = CheckId::new("lockdown.lift");
const MANAGER_SETTINGS: CheckId = CheckId::new("bmc_control.settings");
const AC_CYCLE: CheckId = CheckId::new("power.ac_cycle");
const UEFI_PASSWORD: CheckId = CheckId::new("bios.uefi_password");
const LOCKDOWN_ENABLE: CheckId = CheckId::new("lockdown.enable");
const BMC_RSHIM: CheckId = CheckId::new("dpu.bmc_rshim");
const NIC_MODE: CheckId = CheckId::new("dpu.nic_mode");
const HOST_PRIVILEGE: CheckId = CheckId::new("dpu.host_privilege");
const SECURE_BOOT: CheckId = CheckId::new("secure_boot.enable");
const HTTP_BOOT_ONCE: CheckId = CheckId::new("boot_order.http_boot_once");
const HOST_RSHIM: CheckId = CheckId::new("dpu.host_rshim");
const FACTORY_LOCKDOWN: CheckId = CheckId::new("factory.lockdown");
const FACTORY_UEFI_PASSWORD: CheckId = CheckId::new("factory.uefi_password");
const FACTORY_BIOS: CheckId = CheckId::new("factory.bios");
const FACTORY_BMC: CheckId = CheckId::new("factory.bmc");
const RESET_LOCKDOWN: CheckId = CheckId::new("reset.lockdown");
const RESET_BIOS: CheckId = CheckId::new("reset.bios");
const RESET_BMC: CheckId = CheckId::new("reset.bmc");

pub(super) fn checks(dpu: bool, end_state: EndState) -> Vec<Check> {
    let mut checks = vec![
        Check::apply(ACCOUNT_POLICY, Capability::Accounts, run!(account_policy)),
        apply::accounts_lifecycle(),
        Check::apply(BMC_RESET, Capability::BmcControl, run!(ops::reset_bmc)).once(),
        Check::apply(NTP, Capability::BmcControl, run!(ntp))
            .needs(Needs::NtpServers)
            .after(&[BMC_RESET]),
        Check::apply(UTC, Capability::BmcControl, run!(utc)).after(&[BMC_RESET]),
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
        Check::apply(IPMI_OVER_LAN, Capability::BmcControl, run!(ipmi_over_lan))
            .after(&[BMC_RESET]),
        Check::apply(LOCKDOWN_LIFT, Capability::Lockdown, run!(lift_lockdown))
            .after(&[IPMI_OVER_LAN]),
        Check::apply(
            MANAGER_SETTINGS,
            Capability::BmcControl,
            run!(manager_settings),
        )
        .after(&[LOCKDOWN_LIFT]),
        apply::bios().after(&[LOCKDOWN_LIFT]),
        apply::boot_order().after(&[BIOS]),
        apply::console().after(&[BIOS]),
        apply::power_cycle().after(&[CONSOLE]),
        Check::apply(AC_CYCLE, Capability::Power, run!(ac_cycle))
            .once()
            .needs(Needs::AcCycle)
            .after(&[POWER_CYCLE]),
        uefi_password().after(&[BIOS]),
        Check::apply(LOCKDOWN_ENABLE, Capability::Lockdown, run!(enable_lockdown)).after(&[
            LOCKDOWN_LIFT,
            MANAGER_SETTINGS,
            BIOS,
            BOOT_ORDER,
            CONSOLE,
            POWER_CYCLE,
            AC_CYCLE,
            UEFI_PASSWORD,
        ]),
    ]
}

/// DPU discovery and initialization, with the DPU's own restarts.
fn dpu_setup() -> Vec<Check> {
    vec![
        Check::apply(BMC_RSHIM, Capability::Dpu, run!(bmc_rshim)).after(&[BMC_RESET]),
        Check::apply(NIC_MODE, Capability::Dpu, run!(nic_mode))
            .needs(Needs::HostBmc)
            .after(&[BMC_RESET]),
        Check::apply(HOST_PRIVILEGE, Capability::Dpu, run!(host_privilege)).after(&[NIC_MODE]),
        apply::bios().after(&[HOST_PRIVILEGE]),
        Check::apply(SECURE_BOOT, Capability::SecureBoot, run!(secure_boot)).after(&[BIOS]),
        Check::apply(HTTP_BOOT_ONCE, Capability::BootOrder, run!(http_boot_once))
            .after(&[SECURE_BOOT]),
        Check::apply(HOST_RSHIM, Capability::Dpu, run!(host_rshim)).after(&[NIC_MODE]),
        apply::console(),
        uefi_password().after(&[BIOS]),
    ]
}

/// Back to factory state; each step runs whatever came before.
fn factory_reset() -> Vec<Check> {
    vec![
        Check::apply(FACTORY_LOCKDOWN, Capability::Lockdown, run!(lift_lockdown)),
        Check::apply(
            FACTORY_UEFI_PASSWORD,
            Capability::Bios,
            run!(clear_uefi_password),
        )
        .once()
        .needs(Needs::UefiPassword),
        Check::apply(FACTORY_BIOS, Capability::Bios, run!(reset_bios)),
        Check::apply(FACTORY_BMC, Capability::BmcControl, run!(reset_to_factory)).once(),
    ]
}

/// Back to factory state before validating, for a BMC an earlier run left
/// configured: lockdown off so the BIOS takes the reset, the BMC last since
/// it removes the account the run logged in with.
pub(super) fn factory_reset_first() -> Vec<Check> {
    vec![
        Check::apply(RESET_LOCKDOWN, Capability::Lockdown, run!(lift_lockdown)),
        Check::apply(RESET_BIOS, Capability::Bios, run!(reset_bios)).after(&[RESET_LOCKDOWN]),
        Check::apply(RESET_BMC, Capability::BmcControl, run!(reset_to_factory))
            .once()
            .after(&[RESET_BIOS]),
    ]
}

fn uefi_password() -> Check {
    Check::apply(UEFI_PASSWORD, Capability::Bios, run!(set_uefi_password))
        .once()
        .needs(Needs::UefiPassword)
}

async fn account_policy(cx: &CheckCx<'_>) -> Checked {
    let outcome = requested(
        cx,
        "apply_default_policy",
        cx.bmc().accounts()?.apply_default_policy(),
    )
    .await?;
    ops::wait_for_operations(cx, &outcome).await?;
    let service = async {
        cx.target()
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
    .map_err(failed("reading AccountService"))?;
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
async fn ntp(cx: &CheckCx<'_>) -> Step {
    let servers = &cx.inputs().ntp_servers;
    let bmc = cx.bmc();
    let control = bmc.bmc_control()?;
    let outcome = requested(cx, "set_ntp_servers", control.set_ntp_servers(servers)).await?;
    let listed = ops::read_back(cx, &outcome, Effect::Immediate, async || {
        match control.time_status().await {
            Ok(status) if lists_ntp_servers(&status, servers) => Attempt::Done(status.ntp_servers),
            Ok(status) => Attempt::NotYet(format!(
                "NTP is enabled: {}; it lists {}",
                reported(&status.ntp_enabled),
                status.ntp_servers.join(", ")
            )),
            Err(error) => Attempt::NotYet(describe("time_status", &error)),
        }
    })
    .await?;
    let skew = poll(
        cx,
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

async fn utc(cx: &CheckCx<'_>) -> Step {
    const UTC_OFFSETS: &[&str] = &["+00:00", "-00:00", "Z"];
    let bmc = cx.bmc();
    let control = bmc.bmc_control()?;
    let outcome = requested(cx, "set_utc_timezone", control.set_utc_timezone()).await?;
    let offset = ops::read_back(cx, &outcome, Effect::Immediate, async || {
        match control.time_status().await.map(|status| status.utc_offset) {
            Ok(None) => Attempt::Done(None),
            Ok(Some(offset)) if UTC_OFFSETS.contains(&offset.as_str()) => {
                Attempt::Done(Some(offset))
            }
            Ok(Some(offset)) => Attempt::NotYet(format!("the UTC offset is {offset}")),
            Err(error) => Attempt::NotYet(describe("time_status", &error)),
        }
    })
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

async fn ipmi_over_lan(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let control = bmc.bmc_control()?;
    if control
        .ipmi_over_lan_enabled()
        .await
        .map_err(failed("ipmi_over_lan_enabled"))?
    {
        return Ok("already enabled".to_string());
    }
    let outcome = requested(cx, "set_ipmi_over_lan", control.set_ipmi_over_lan(true)).await?;
    ops::read_back(cx, &outcome, Effect::Immediate, async || {
        match control.ipmi_over_lan_enabled().await {
            Ok(true) => Attempt::Done(()),
            Ok(false) => Attempt::NotYet("IPMI over LAN reads disabled".to_string()),
            Err(error) => Attempt::NotYet(describe("ipmi_over_lan_enabled", &error)),
        }
    })
    .await?;
    Ok(format!(
        "enabled ({}); IPMI over LAN reads enabled",
        how(&outcome)
    ))
}

async fn lift_lockdown(cx: &CheckCx<'_>) -> Step {
    ops::set_lockdown(cx, LockdownDesiredState::Disabled).await
}

async fn enable_lockdown(cx: &CheckCx<'_>) -> Step {
    ops::set_lockdown(cx, LockdownDesiredState::Enabled).await
}

async fn manager_settings(cx: &CheckCx<'_>) -> Checked {
    let bmc = cx.bmc();
    let control = bmc.bmc_control()?;
    let outcome = requested(
        cx,
        "apply_settings",
        control.apply_settings(&ManagerSettings::default()),
    )
    .await?;
    let status = ops::read_back(cx, &outcome, Effect::Immediate, async || {
        match control.settings_status().await {
            Ok(status) if status.is_applied => Attempt::Done(status),
            Ok(status) => Attempt::NotYet(format!("{} settings differ", status.differences.len())),
            Err(error) => Attempt::NotYet(describe("settings_status", &error)),
        }
    })
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
async fn ac_cycle(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let power = bmc.power()?;
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
    let outcome = ops::sent_or_dropped(
        cx,
        "power FullPowerCycle",
        power.set(ResetType::FullPowerCycle),
    )
    .await?;
    ops::wait_for_bmc_down(cx, ops::ac_cycle_down_timeout(&outcome))
        .await
        .map_err(|outcome| match outcome {
            Outcome::Fail(reason) => {
                Outcome::Fail(format!("no power cycle was observed: {reason}"))
            }
            other => other,
        })?;
    ops::wait_for_bmc_up(cx).await?;
    ops::settle(cx, &outcome).await?;
    ops::power_on(cx).await?;
    Ok(format!(
        "the BMC lost power, and everything was back on {} after the request ({})",
        took(started),
        how(&outcome)
    ))
}

/// Sets the UEFI password and restarts the host so it applies. The password
/// cannot be read back, so an accepted request is taken as set.
async fn set_uefi_password(cx: &CheckCx<'_>) -> Step {
    let password = cx.inputs().required_uefi_password()?;
    let outcome = requested(
        cx,
        "change_uefi_password",
        cx.bmc()
            .bios()?
            .change_uefi_password(&password.current, &password.new),
    )
    .await?;
    ops::take_effect(cx, &outcome).await?;
    Ok(format!("set ({}) and restarted", how(&outcome)))
}

async fn clear_uefi_password(cx: &CheckCx<'_>) -> Step {
    let password = cx.inputs().required_uefi_password()?;
    let outcome = requested(
        cx,
        "clear_uefi_password",
        cx.bmc().bios()?.clear_uefi_password(&password.new),
    )
    .await?;
    ops::take_effect(cx, &outcome).await?;
    Ok(format!("cleared ({}) and restarted", how(&outcome)))
}

/// Restores the BIOS defaults and restarts the host; the reset shows as the
/// settings the validator applies no longer being in effect.
async fn reset_bios(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let bios = bmc.bios()?;
    let outcome = requested(cx, "reset", bios.reset()).await?;
    let inputs = cx.inputs();
    let boot_interface = inputs.boot_interface();
    let differing = ops::read_back(cx, &outcome, Effect::OnRestart, async || {
        match bios
            .status(&inputs.bios_profile, boot_interface.as_ref())
            .await
        {
            Ok(status) if !status.is_applied => Attempt::Done(status.differences.len()),
            Ok(_) => {
                Attempt::NotYet("the validator's BIOS settings are still in effect".to_string())
            }
            Err(error) => Attempt::NotYet(describe("status", &error)),
        }
    })
    .await?;
    Ok(format!(
        "reset to defaults ({}) and restarted; {} now differ from what the validator applies",
        how(&outcome),
        counted(differing, "setting", "settings")
    ))
}

/// Resets the BMC to factory defaults, then waits for it to go down and
/// come back answering to the factory password.
async fn reset_to_factory(cx: &CheckCx<'_>) -> Step {
    let factory = cx.inputs().factory.as_ref().ok_or_else(|| {
        Outcome::Skipped("the factory credentials are only known to --validate".to_string())
    })?;
    let started = Instant::now();
    let outcome = ops::sent_or_dropped(
        cx,
        "reset_to_factory_defaults",
        cx.bmc().bmc_control()?.reset_to_factory_defaults(),
    )
    .await?;
    ops::wait_for_bmc_down(cx, FACTORY_DOWN_TIMEOUT).await?;
    poll(
        cx,
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
        "the BMC went down and answered to the factory password {} after the request ({})",
        took(started),
        how(&outcome)
    ))
}

async fn bmc_rshim(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let dpu = bmc.dpu()?;
    let outcome = requested(cx, "enable_bmc_rshim", dpu.enable_bmc_rshim()).await?;
    ops::read_back(cx, &outcome, Effect::Immediate, async || {
        match dpu.status().await {
            Ok(status) if status.bmc_rshim == Some(RshimState::Enabled) => Attempt::Done(()),
            Ok(status) => {
                Attempt::NotYet(format!("BMC rshim reads {}", reported(&status.bmc_rshim)))
            }
            Err(error) => Attempt::NotYet(describe("status", &error)),
        }
    })
    .await?;
    Ok(format!("BMC rshim reads Enabled ({})", how(&outcome)))
}

/// Switches the DPU to DPU mode, which takes a host power cycle.
async fn nic_mode(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let dpu = bmc.dpu()?;
    let status = dpu.status().await.map_err(failed("status"))?;
    if status.nic_mode == Some(NicMode::Dpu) {
        return Ok("already in DPU mode".to_string());
    }
    let host = cx.inputs().required_host_bmc()?;
    let started = Instant::now();
    let outcome = requested(cx, "set_nic_mode", dpu.set_nic_mode(NicMode::Dpu)).await?;
    ops::settle(cx, &outcome).await?;
    let host = connect_host(host).await?;
    cx.trace("power-cycling the host through its BMC");
    ops::set_power(cx, &host, ResetType::ForceOff).await?;
    ops::set_power(cx, &host, ResetType::On).await?;
    ops::wait_for_bmc_up(cx).await?;
    let bmc = cx.bmc();
    let dpu = bmc.dpu()?;
    poll(
        cx,
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

async fn host_privilege(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let dpu = bmc.dpu()?;
    let outcome = requested(
        cx,
        "set_host_privilege_level",
        dpu.set_host_privilege_level(HostPrivilegeLevel::Restricted),
    )
    .await?;
    ops::read_back(cx, &outcome, Effect::OnRestart, async || {
        match dpu.status().await {
            Ok(status) if status.host_privilege == Some(HostPrivilegeLevel::Restricted) => {
                Attempt::Done(())
            }
            Ok(status) => Attempt::NotYet(format!(
                "host privilege reads {}",
                reported(&status.host_privilege)
            )),
            Err(error) => Attempt::NotYet(describe("status", &error)),
        }
    })
    .await?;
    Ok(format!(
        "host privilege reads Restricted after a DPU restart ({})",
        how(&outcome)
    ))
}

async fn secure_boot(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let secure_boot = bmc.secure_boot()?;
    let mut outcome = requested(
        cx,
        "set",
        secure_boot.set(
            &SecureBootUpdate::builder()
                .with_secure_boot_enable(true)
                .build(),
        ),
    )
    .await?;
    let platform_key = cx.inputs().platform_key.as_ref();
    if let Some(key) = platform_key {
        outcome = outcome
            .merge(requested(cx, "add_platform_key", secure_boot.add_platform_key(key)).await?);
    }
    let want_key = platform_key.is_some();
    ops::read_back(cx, &outcome, Effect::OnRestart, async || {
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
    cx: &CheckCx<'_>,
) -> Result<(Option<BootSource>, Option<BootSourceOverrideEnabled>), PlatformError> {
    let connection = cx.target().fresh_connection().await?;
    let bmc = cx.bmc();
    let op = OpCx::new(
        connection.bmc.as_ref(),
        connection.service_root.as_ref(),
        bmc.endpoint().identity(),
    );
    let raw = op.system().await?.raw();
    let boot = raw.boot.as_ref();
    Ok((
        boot.and_then(|boot| boot.boot_source_override_target.flatten()),
        boot.and_then(|boot| boot.boot_source_override_enabled.flatten()),
    ))
}

/// Sets a one-time UEFI HTTP boot, restarts the DPU, and checks the
/// override was consumed.
async fn http_boot_once(cx: &CheckCx<'_>) -> Step {
    let http_once = (
        Some(BootSource::UefiHttp),
        Some(BootSourceOverrideEnabled::Once),
    );
    let outcome = requested(
        cx,
        "set_override",
        cx.bmc().boot_order()?.set_override(
            &BootUpdate::builder()
                .with_boot_source_override_target(BootSource::UefiHttp)
                .with_boot_source_override_enabled(BootSourceOverrideEnabled::Once)
                .build(),
        ),
    )
    .await?;
    ops::read_back(
        cx,
        &outcome,
        Effect::Immediate,
        async || match boot_override(cx).await {
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
    ops::restart_host(cx).await?;
    poll(
        cx,
        SETTLE_TIMEOUT,
        SETTLE_POLL_INTERVAL,
        async || match boot_override(cx).await {
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

async fn host_rshim(cx: &CheckCx<'_>) -> Step {
    let bmc = cx.bmc();
    let dpu = bmc.dpu()?;
    let outcome = requested(
        cx,
        "set_host_rshim",
        dpu.set_host_rshim(RshimState::Enabled),
    )
    .await?;
    ops::read_back(cx, &outcome, Effect::Immediate, async || {
        match dpu.status().await {
            Ok(status) if status.host_rshim == Some(RshimState::Enabled) => Attempt::Done(()),
            Ok(status) => {
                Attempt::NotYet(format!("host rshim reads {}", reported(&status.host_rshim)))
            }
            Err(error) => Attempt::NotYet(describe("status", &error)),
        }
    })
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
        let sequences = [
            checks(false, EndState::Factory),
            checks(true, EndState::Factory),
            factory_reset_first(),
        ];
        for checks in sequences {
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
                        "{} follows {dependency}, which must run once, before it",
                        check.id
                    );
                }
            }
        }
    }
}
