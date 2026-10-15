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
#![cfg_attr(not(test), deny(dead_code_pub_in_binary))]

//! Validates the compiled BMC drivers against a real BMC.
//!
//! The BMC is discovered, drivers are selected by the compiled rules, and
//! the connection is built by `bmc-runtime` exactly as a controller would;
//! checks then call the selected drivers the way controllers do.

mod checks;
mod connect;
mod ownership;
mod report;
mod selection;

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use bmc_platform::{BiosSettings, Capability};
use clap::Parser;
use colored::Colorize;
use mac_address::MacAddress;

use crate::checks::{Ctx, EndState, Inputs, Scope, UefiPassword};
use crate::connect::Target;
use crate::report::Redactor;
use crate::selection::Selection;

/// The Redfish port a BMC address without one uses.
const HTTPS_PORT: u16 = 443;

#[derive(Debug, Parser)]
#[command(
    name = "bmc-validate",
    about = "Validate the compiled BMC drivers against a real BMC."
)]
struct Args {
    /// IP address of the BMC.
    bmc_ip: IpAddr,

    /// HTTPS port of the BMC's Redfish service.
    #[arg(long, default_value_t = HTTPS_PORT)]
    port: u16,

    /// BMC account to log in with.
    #[arg(long, short)]
    username: String,

    /// Password of that account; with `--validate`, its factory password.
    /// Prefer the environment variable: command arguments are visible to
    /// other users of the machine.
    #[arg(long, env = "BMC_PASSWORD", hide_env_values = true)]
    password: String,

    /// Pin a capability to a driver, as `<capability>=<driver-id>` or a
    /// driver id alone. `<capability>=standard` and
    /// `<capability>=unsupported` are accepted. Repeatable.
    #[arg(long = "driver", value_name = "[CAPABILITY=]DRIVER")]
    drivers: Vec<String>,

    /// Also run checks that change the BMC and host: stage BIOS, console,
    /// and boot-order settings and restart the host to apply them (leaving
    /// them applied, and clearing pending iDRAC jobs), cycle host power,
    /// toggle lockdown, and create and delete a temporary account.
    #[arg(long)]
    apply: bool,

    /// Take a BMC from factory state through everything NICo does to it,
    /// in production order.
    #[arg(long, conflicts_with_all = ["apply", "only", "checks"], requires = "new_password")]
    validate: bool,

    /// Run only the checks of these capabilities.
    #[arg(long, value_delimiter = ',')]
    only: Vec<Capability>,

    /// Run only these checks; combines with `--only`.
    #[arg(long = "check", value_delimiter = ',', value_name = "CHECK_ID")]
    checks: Vec<String>,

    /// Print the driver map and the planned checks without running them.
    #[arg(long)]
    plan: bool,

    /// MAC address of the host interface boot order should put first;
    /// boot-order checks need it.
    #[arg(long)]
    boot_mac: Option<MacAddress>,

    /// JSON object of BIOS attributes to apply and check on top of the
    /// platform's own settings.
    #[arg(long)]
    bios_profile: Option<PathBuf>,

    /// Attach IPMI through `ipmitool` on port 623, for drivers that restart
    /// or reset over IPMI.
    #[arg(long)]
    ipmi: bool,

    /// Most read checks in flight at once.
    #[arg(long, default_value_t = 4)]
    jobs: usize,

    /// Print each step of the apply checks to stderr as it happens: driver
    /// calls and how they completed, job status, restarts, and status reads.
    #[arg(long, short)]
    verbose: bool,

    /// The password `--validate` sets on `--username` in place of the
    /// factory one.
    #[arg(
        long,
        env = "BMC_NEW_PASSWORD",
        hide_env_values = true,
        requires = "validate"
    )]
    new_password: Option<String>,

    /// What `--validate` leaves the BMC in.
    #[arg(long, value_enum, default_value_t, requires = "validate")]
    end_state: EndState,

    /// NTP server the BMC should sync from. Repeatable.
    #[arg(long = "ntp-server", value_name = "SERVER", requires = "validate")]
    ntp_servers: Vec<String>,

    /// UEFI setup password `--validate` sets.
    #[arg(
        long,
        env = "BMC_UEFI_PASSWORD",
        hide_env_values = true,
        requires = "validate"
    )]
    uefi_password: Option<String>,

    /// The UEFI setup password the firmware has now; a BlueField's is its
    /// factory default.
    #[arg(
        long,
        env = "BMC_CURRENT_UEFI_PASSWORD",
        hide_env_values = true,
        default_value = "",
        requires = "uefi_password"
    )]
    current_uefi_password: String,

    /// Let `--validate` cut and restore AC power to the host.
    #[arg(long, requires = "validate")]
    allow_ac_cycle: bool,

    /// PEM file of the platform key secure boot enrolls on a DPU.
    #[arg(long, requires = "validate")]
    pk_pem: Option<PathBuf>,

    /// The host BMC of a DPU under `--validate`, as `IP` or `IP:PORT`; it
    /// power-cycles the host when the DPU changes NIC mode.
    #[arg(long, value_parser = bmc_address, requires_all = ["validate", "host_username", "host_password"])]
    host_bmc: Option<SocketAddr>,

    /// Account on the host BMC.
    #[arg(long, requires = "host_bmc")]
    host_username: Option<String>,

    /// Password of that account.
    #[arg(
        long,
        env = "BMC_HOST_PASSWORD",
        hide_env_values = true,
        requires = "host_bmc"
    )]
    host_password: Option<String>,
}

fn bmc_address(value: &str) -> Result<SocketAddr, String> {
    value
        .parse()
        .or_else(|_| value.parse().map(|ip| SocketAddr::new(ip, HTTPS_PORT)))
        .map_err(|_| format!("{value} is neither an IP address nor IP:PORT"))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    report::init();
    let redactor = Arc::new(Redactor::new(
        [
            Some(args.password.as_str()),
            args.new_password.as_deref(),
            args.uefi_password.as_deref(),
            Some(args.current_uefi_password.as_str()),
            args.host_password.as_deref(),
        ]
        .into_iter()
        .flatten(),
    ));
    match run(args, redactor.clone()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{} {}", "error:".red().bold(), redactor.apply(&error));
            ExitCode::from(2)
        }
    }
}

async fn run(args: Args, redactor: Arc<Redactor>) -> Result<ExitCode, String> {
    let address = SocketAddr::new(args.bmc_ip, args.port);
    let given = Target::new(address, &args.username, &args.password);
    let (target, factory, factory_accounts) = match &args.new_password {
        Some(new_password) => {
            let (owned, accounts) = own(&args, &given, new_password).await?;
            (owned, Some(given), accounts)
        }
        None => (given, None, None),
    };

    report::discovering(address);
    let identity = target.discover().await?;
    let selection = Selection::resolve(&identity, &args.drivers)?;
    report::print_identity(address, &identity);
    report::print_selection(&selection);
    if let Some(driver) = factory_accounts
        && driver != selection.driver(Capability::Accounts)
    {
        report::warn(&format!(
            "the factory password was changed through {driver}, but the full identity selects {}",
            selection.driver(Capability::Accounts)
        ));
    }

    let inputs = Inputs {
        ipmi: args.ipmi,
        boot_mac: args.boot_mac,
        bios_profile: bios_profile(args.bios_profile.as_deref())?,
        ntp_servers: args.ntp_servers,
        uefi_password: args.uefi_password.map(|new| UefiPassword {
            current: args.current_uefi_password,
            new,
        }),
        allow_ac_cycle: args.allow_ac_cycle,
        platform_key: args
            .pk_pem
            .as_deref()
            .map(|path| read(path, "the platform key"))
            .transpose()?,
        host_bmc: args.host_bmc.map(|address| {
            Target::new(
                address,
                args.host_username.as_deref().unwrap_or_default(),
                args.host_password.as_deref().unwrap_or_default(),
            )
        }),
        factory,
        verbose: args.verbose,
    };
    let scope = if args.validate {
        Scope::Validate(args.end_state)
    } else {
        Scope::Checks {
            apply: args.apply,
            only: &args.only,
            ids: &args.checks,
        }
    };
    let plan = checks::plan(&selection, &inputs, scope)?;
    if args.plan {
        report::print_plan(&plan, &selection);
        return Ok(ExitCode::SUCCESS);
    }

    let bmc = target
        .connect(selection.endpoint(address, identity)?, args.ipmi)
        .await?;
    let ctx = Ctx::new(bmc, selection, target, inputs, redactor);
    report::checks_heading();
    let started = Instant::now();
    let results = checks::run(&ctx, plan, args.jobs.max(1)).await;
    Ok(report::summary(&ctx, &results, started.elapsed()))
}

/// The BMC reached with `new_password`, replacing the factory password
/// first unless only planning: a plan changes nothing, so it reaches the
/// BMC with whichever password it accepts. Also returns the accounts driver
/// that replaced the password, if one did.
async fn own(
    args: &Args,
    factory: &Target,
    new_password: &str,
) -> Result<(Target, Option<String>), String> {
    let owned = factory.as_account(factory.username(), new_password);
    if args.plan {
        let reachable = match owned.log_in().await {
            Ok(()) => owned,
            Err(_) => factory.clone(),
        };
        return Ok((reachable, None));
    }
    report::taking_ownership(factory.address());
    let (owned, ownership) = ownership::take(factory, new_password, &args.drivers).await?;
    report::print_ownership(&ownership);
    Ok((owned, ownership.accounts_driver))
}

fn bios_profile(path: Option<&Path>) -> Result<BiosSettings, String> {
    let Some(path) = path else {
        return Ok(BiosSettings::default());
    };
    Ok(BiosSettings {
        attributes: serde_json::from_str::<BTreeMap<_, _>>(&read(path, "the BIOS profile")?)
            .map_err(|error| format!("{} is not a JSON object: {error}", path.display()))?,
    })
}

fn read(path: &Path, what: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("reading {what} {}: {error}", path.display()))
}
