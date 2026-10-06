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

//! Checks that only read: every read operation of every capability.

use std::time::{Duration, Instant};

use bmc_platform::{Capability, ConsoleSpec, EvidenceProgress, PlatformError};

use super::{Check, Ctx, Outcome, Step, differences, failed};

/// How long a signed-measurement request may stay pending.
const EVIDENCE_TIMEOUT: Duration = Duration::from_secs(180);
const EVIDENCE_POLL_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn checks() -> Vec<Check> {
    vec![
        Check::read("power.state", Capability::Power, |ctx| {
            Box::pin(power_state(ctx))
        }),
        Check::read("power.ac_power_cycle_supported", Capability::Power, |ctx| {
            Box::pin(ac_power_cycle_supported(ctx))
        }),
        Check::read("bmc_control.ipmi_over_lan", Capability::BmcControl, |ctx| {
            Box::pin(ipmi_over_lan(ctx))
        }),
        Check::read(
            "bmc_control.settings_status",
            Capability::BmcControl,
            |ctx| Box::pin(manager_settings_status(ctx)),
        ),
        Check::read("bios.status", Capability::Bios, |ctx| {
            Box::pin(bios_status(ctx))
        }),
        Check::read("bios.infinite_boot", Capability::Bios, |ctx| {
            Box::pin(infinite_boot(ctx))
        }),
        Check::read("boot_order.status", Capability::BootOrder, |ctx| {
            Box::pin(boot_order_status(ctx))
        })
        .needs_boot_interface(),
        Check::read("secure_boot.status", Capability::SecureBoot, |ctx| {
            Box::pin(secure_boot_status(ctx))
        }),
        Check::read("secure_boot.platform_key", Capability::SecureBoot, |ctx| {
            Box::pin(platform_key(ctx))
        }),
        Check::read("lockdown.status", Capability::Lockdown, |ctx| {
            Box::pin(lockdown_status(ctx))
        }),
        Check::read("accounts.list", Capability::Accounts, |ctx| {
            Box::pin(accounts_list(ctx))
        }),
        Check::read("firmware.inventory", Capability::Firmware, |ctx| {
            Box::pin(firmware_inventory(ctx))
        }),
        Check::read("storage.boot_controller", Capability::Storage, |ctx| {
            Box::pin(boot_controller(ctx))
        }),
        Check::read("dpu.status", Capability::Dpu, |ctx| {
            Box::pin(dpu_status(ctx))
        }),
        Check::read("attestation.components", Capability::Attestation, |ctx| {
            Box::pin(attestation_components(ctx))
        }),
        Check::read("attestation.evidence", Capability::Attestation, |ctx| {
            Box::pin(attestation_evidence(ctx))
        })
        .timeout(EVIDENCE_TIMEOUT + Duration::from_secs(60)),
        Check::read("console.status", Capability::Console, |ctx| {
            Box::pin(console_status(ctx))
        }),
        Check::read("console.spec", Capability::Console, |ctx| {
            Box::pin(console_spec(ctx))
        }),
    ]
}

async fn power_state(ctx: &Ctx) -> Step {
    let state = ctx
        .bmc
        .drivers()
        .power()?
        .state(&ctx.bmc.operation_context())
        .await
        .map_err(failed("state"))?;
    Ok(match state {
        Some(state) => format!("{state:?}"),
        None => "the platform reports no single power state".to_string(),
    })
}

async fn ac_power_cycle_supported(ctx: &Ctx) -> Step {
    let supported = ctx
        .bmc
        .drivers()
        .power()?
        .ac_power_cycle_supported(&ctx.bmc.operation_context())
        .await
        .map_err(failed("ac_power_cycle_supported"))?;
    Ok(format!("supported={supported}"))
}

async fn ipmi_over_lan(ctx: &Ctx) -> Step {
    let enabled = ctx
        .bmc
        .drivers()
        .bmc_control()?
        .ipmi_over_lan_enabled(&ctx.bmc.operation_context())
        .await
        .map_err(failed("ipmi_over_lan_enabled"))?;
    Ok(format!("enabled={enabled}"))
}

async fn manager_settings_status(ctx: &Ctx) -> Step {
    let status = ctx
        .bmc
        .drivers()
        .bmc_control()?
        .settings_status(&ctx.bmc.operation_context())
        .await
        .map_err(failed("settings_status"))?;
    Ok(differences(status.differences.iter().map(|diff| {
        (diff.key.as_str(), &diff.expected, diff.actual.as_ref())
    })))
}

async fn bios_status(ctx: &Ctx) -> Step {
    let status = ctx
        .bmc
        .drivers()
        .bios()?
        .status(
            &ctx.bmc.operation_context(),
            &ctx.inputs.bios_profile,
            ctx.inputs.boot_interface.as_ref(),
        )
        .await
        .map_err(failed("status"))?;
    Ok(differences(status.differences.iter().map(|diff| {
        (diff.key.as_str(), &diff.expected, diff.actual.as_ref())
    })))
}

async fn infinite_boot(ctx: &Ctx) -> Step {
    let enabled = ctx
        .bmc
        .drivers()
        .bios()?
        .infinite_boot_enabled(&ctx.bmc.operation_context())
        .await
        .map_err(failed("infinite_boot_enabled"))?;
    Ok(match enabled {
        Some(enabled) => format!("enabled={enabled}"),
        None => "the platform has no infinite-boot setting".to_string(),
    })
}

async fn boot_order_status(ctx: &Ctx) -> Step {
    let selector = ctx
        .inputs
        .boot_interface
        .as_ref()
        .ok_or_else(|| Outcome::Skipped("needs --boot-mac".to_string()))?;
    let status = ctx
        .bmc
        .drivers()
        .boot_order()?
        .status(&ctx.bmc.operation_context(), selector)
        .await
        .map_err(failed("status"))?;
    Ok(format!("{status:?}"))
}

async fn secure_boot_status(ctx: &Ctx) -> Step {
    let status = ctx
        .bmc
        .drivers()
        .secure_boot()?
        .status(&ctx.bmc.operation_context())
        .await
        .map_err(failed("status"))?;
    Ok(format!("{status:?}"))
}

async fn platform_key(ctx: &Ctx) -> Step {
    let present = ctx
        .bmc
        .drivers()
        .secure_boot()?
        .has_platform_key(&ctx.bmc.operation_context())
        .await
        .map_err(failed("has_platform_key"))?;
    Ok(format!("platform key present={present}"))
}

async fn lockdown_status(ctx: &Ctx) -> Step {
    let status = ctx
        .bmc
        .drivers()
        .lockdown()?
        .status(&ctx.bmc.operation_context())
        .await
        .map_err(failed("status"))?;
    Ok(format!(
        "aggregate={:?} host={:?} bmc={:?} ({})",
        status.aggregate, status.host, status.bmc, status.message
    ))
}

/// The account this run logged in with must be listed.
async fn accounts_list(ctx: &Ctx) -> Step {
    let accounts = ctx
        .bmc
        .drivers()
        .accounts()?
        .list(&ctx.bmc.operation_context())
        .await
        .map_err(failed("list"))?;
    let names: Vec<&str> = accounts
        .iter()
        .filter_map(|account| account.user_name.as_deref())
        .filter(|name| !name.is_empty())
        .collect();
    if !names.contains(&ctx.inputs.username.as_str()) {
        return Err(Outcome::Fail(format!(
            "the account this run logged in with is not listed; listed: {}",
            names.join(", ")
        )));
    }
    Ok(format!("{} accounts: {}", names.len(), names.join(", ")))
}

async fn firmware_inventory(ctx: &Ctx) -> Step {
    let inventory = ctx
        .bmc
        .drivers()
        .firmware()?
        .inventory(&ctx.bmc.operation_context())
        .await
        .map_err(failed("inventory"))?;
    let entries: Vec<String> = inventory
        .iter()
        .map(|entry| {
            let version = entry.version.clone().flatten();
            format!("{}={}", entry.id, version.as_deref().unwrap_or("?"))
        })
        .collect();
    Ok(format!("{} entries: {}", entries.len(), entries.join(", ")))
}

async fn boot_controller(ctx: &Ctx) -> Step {
    let controller = ctx
        .bmc
        .drivers()
        .storage()?
        .boot_controller(&ctx.bmc.operation_context())
        .await
        .map_err(failed("boot_controller"))?;
    Ok(match controller {
        Some(id) => format!("boot controller {id}"),
        None => "no boot controller".to_string(),
    })
}

async fn dpu_status(ctx: &Ctx) -> Step {
    let status = ctx
        .bmc
        .drivers()
        .dpu()?
        .status(&ctx.bmc.operation_context())
        .await
        .map_err(failed("status"))?;
    Ok(format!(
        "nic_mode={:?} host_rshim={:?}",
        status.nic_mode, status.host_rshim
    ))
}

async fn attestation_components(ctx: &Ctx) -> Step {
    let components = ctx
        .bmc
        .drivers()
        .attestation()?
        .components(&ctx.bmc.operation_context())
        .await
        .map_err(failed("components"))?;
    let ids: Vec<&str> = components
        .iter()
        .map(|component| component.id.as_str())
        .collect();
    Ok(format!("{} components: {}", ids.len(), ids.join(", ")))
}

/// Collects signed measurements from the first component, with its
/// certificate and firmware.
async fn attestation_evidence(ctx: &Ctx) -> Step {
    let attestation = ctx.bmc.drivers().attestation()?;
    let components = attestation
        .components(&ctx.bmc.operation_context())
        .await
        .map_err(failed("components"))?;
    let Some(component) = components.first() else {
        return Err(Outcome::Skipped(
            "the BMC lists no ComponentIntegrity resources".to_string(),
        ));
    };
    let id = component.id.as_str();
    let certificate = attestation
        .ca_certificate(&ctx.bmc.operation_context(), id)
        .await
        .map_err(failed("ca_certificate"))?;
    let firmware = match attestation
        .firmware_for_component(&ctx.bmc.operation_context(), id)
        .await
    {
        Ok(firmware) => firmware
            .version
            .clone()
            .flatten()
            .unwrap_or_else(|| firmware.id.clone()),
        Err(PlatformError::Unsupported) => "not linked on this platform".to_string(),
        Err(error) => return Err(failed("firmware_for_component")(error)),
    };

    let nonce: [u8; 32] = rand::random();
    let mut progress = attestation
        .request_evidence(&ctx.bmc.operation_context(), id, &nonce)
        .await
        .map_err(failed("request_evidence"))?;
    let deadline = Instant::now() + EVIDENCE_TIMEOUT;
    loop {
        match progress {
            EvidenceProgress::Ready(_) => {
                return Ok(format!(
                    "signed measurements from {id}; certificate {}, firmware {firmware}",
                    certificate.id
                ));
            }
            EvidenceProgress::Failed { state, messages } => {
                let messages: Vec<&str> = messages
                    .iter()
                    .filter_map(|message| message.message.as_deref())
                    .collect();
                return Err(Outcome::Fail(format!(
                    "measurement collection for {id} ended {state:?}: {}",
                    messages.join("; ")
                )));
            }
            EvidenceProgress::Pending(reference) => {
                if Instant::now() + EVIDENCE_POLL_INTERVAL > deadline {
                    return Err(Outcome::Fail(format!(
                        "measurement collection {} still pending after {}s",
                        reference.uri(),
                        EVIDENCE_TIMEOUT.as_secs()
                    )));
                }
                tokio::time::sleep(EVIDENCE_POLL_INTERVAL).await;
                progress = attestation
                    .poll_evidence(&ctx.bmc.operation_context(), &reference)
                    .await
                    .map_err(failed("poll_evidence"))?;
            }
        }
    }
}

async fn console_status(ctx: &Ctx) -> Step {
    let status = ctx
        .bmc
        .drivers()
        .console()?
        .status(&ctx.bmc.operation_context())
        .await
        .map_err(failed("status"))?;
    Ok(format!("{:?} ({})", status.state, status.message))
}

async fn console_spec(ctx: &Ctx) -> Step {
    let spec = ctx
        .bmc
        .drivers()
        .console()?
        .spec(&ctx.bmc.operation_context())
        .await
        .map_err(failed("spec"))?;
    Ok(match spec {
        ConsoleSpec::SshShell(shell) => format!("SSH shell on port {}", shell.port),
        ConsoleSpec::SshDirect { port } => format!("direct SSH on port {port}"),
        ConsoleSpec::IpmiSol { port, .. } => format!("IPMI SOL on port {port}"),
        ConsoleSpec::None { reason } => format!("no console transport: {reason}"),
    })
}
