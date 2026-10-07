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
mod report;
mod selection;

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bmc_platform::{BiosSettings, BootInterfaceSelector, Capability};
use bmc_runtime::ConnectionManager;
use clap::Parser;
use colored::Colorize;
use mac_address::MacAddress;
use nv_redfish::bmc_http::BmcCredentials;

use crate::checks::{Ctx, Inputs};
use crate::report::Redactor;

/// How long discovery may take before the run gives up.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Parser)]
#[command(
    name = "bmc-validate",
    about = "Validate the compiled BMC drivers against a real BMC."
)]
struct Args {
    /// IP address of the BMC.
    bmc_ip: IpAddr,

    /// HTTPS port of the BMC's Redfish service.
    #[arg(long, default_value_t = 443)]
    port: u16,

    /// BMC account to log in with.
    #[arg(long, short)]
    username: String,

    /// Password of that account. Prefer the environment variable: command
    /// arguments are visible to other users of the machine.
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
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    report::init();
    let redactor = Arc::new(Redactor::new(&args.password));
    match run(args, redactor.clone()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{} {}", "error:".red().bold(), redactor.apply(&error));
            ExitCode::from(2)
        }
    }
}

async fn run(args: Args, redactor: Arc<Redactor>) -> Result<ExitCode, String> {
    let bios_profile = match &args.bios_profile {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            BiosSettings {
                attributes: serde_json::from_str::<BTreeMap<_, _>>(&text)
                    .map_err(|error| format!("{} is not a JSON object: {error}", path.display()))?,
            }
        }
        None => BiosSettings::default(),
    };

    let address = SocketAddr::new(args.bmc_ip, args.port);
    let credentials = BmcCredentials::new(args.username.clone(), args.password.clone());
    let pool = connect::pool();

    report::discovering(address);
    let identity = tokio::time::timeout(
        DISCOVERY_TIMEOUT,
        connect::discover(&pool, address, credentials.clone()),
    )
    .await
    .map_err(|_| format!("discovery did not finish within {DISCOVERY_TIMEOUT:?}"))??;
    let selection = selection::Selection::resolve(&identity, &args.drivers)?;
    report::print_identity(address, &identity);
    report::print_selection(&selection);

    let plan = checks::plan(
        &selection,
        args.apply,
        &args.only,
        &args.checks,
        args.boot_mac.is_some(),
    )?;
    if args.plan {
        report::print_plan(&plan, &selection);
        return Ok(ExitCode::SUCCESS);
    }

    let ipmi = args
        .ipmi
        .then(|| connect::ipmi(args.bmc_ip, &args.username, &args.password));
    let manager = ConnectionManager::new(
        pool.clone(),
        connect::static_credentials(credentials),
        "bmc-validate".to_string(),
    )
    .map_err(|error| error.to_string())?;
    let bmc = manager
        .connect(selection.endpoint(address, identity)?, ipmi)
        .await
        .map_err(|error| format!("connecting through bmc-runtime: {error}"))?;

    let inputs = Inputs {
        username: args.username,
        ipmi: args.ipmi,
        boot_interface: args.boot_mac.map(BootInterfaceSelector::Mac),
        bios_profile,
    };
    let ctx = Ctx::new(bmc, pool, address, inputs, redactor);
    report::checks_heading();
    let started = Instant::now();
    let results = checks::run(&ctx, plan, &selection, args.jobs.max(1)).await;
    Ok(report::summary(&ctx, &results, started.elapsed()))
}
