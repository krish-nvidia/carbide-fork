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

//! Read-only smoke test against a real BMC.
//!
//! Builds the platform runtime with a fixed-credential provider, prints which
//! plugin is selected, then runs every **read-only** capability the selected
//! plugin implements (status/inventory reads only -- no mutations), reporting
//! `Ok`/`NotSupported`/error per capability. Use it to confirm the runtime,
//! selection, and a plugin's read paths work against live hardware.
//!
//! Usage:
//!   cargo run -p carbide-redfish-platform-runtime --example probe-bmc -- \
//!       <ip:port> <username> <password>

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use carbide_redfish_platform_api::error::RedfishError;
use carbide_redfish_platform_api::model::{BmcEndpointKind, BmcRef};
use carbide_redfish_platform_runtime::{BmcCredentialProvider, BmcCredentials, build_runtime};

struct DirectCredentials {
    username: String,
    password: String,
}

#[async_trait]
impl BmcCredentialProvider for DirectCredentials {
    async fn credentials_for(&self, _bmc: &BmcRef) -> Result<BmcCredentials, RedfishError> {
        Ok(BmcCredentials {
            username: self.username.clone(),
            password: self.password.clone(),
        })
    }
}

/// Print the result of one read, tolerating `NotSupported` so the sweep
/// continues across capabilities the plugin does not implement.
fn report<T: std::fmt::Debug>(label: &str, result: Result<T, RedfishError>) {
    match result {
        Ok(value) => println!("  [ ok ] {label}: {value:?}"),
        Err(RedfishError::NotSupported(why)) => println!("  [skip] {label}: not supported ({why})"),
        Err(err) => println!("  [FAIL] {label}: {err}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: probe-bmc <ip:port> <username> <password>";
    let address: SocketAddr = args.next().ok_or(usage)?.parse()?;
    let username = args.next().ok_or(usage)?;
    let password = args.next().ok_or(usage)?;

    let credentials: Arc<dyn BmcCredentialProvider> =
        Arc::new(DirectCredentials { username, password });
    let service = build_runtime(credentials);
    let bmc = BmcRef::new(address, BmcEndpointKind::HostBmc);

    let selected = service.selected_platform(bmc.clone()).await?;
    println!(
        "selected plugin: {} (vendor: {}, specificity: {:?})",
        selected.plugin_id, selected.vendor, selected.specificity
    );
    println!("identity: {:#?}", selected.identity);

    println!("\nread-only capability sweep:");
    report("power_state", service.power_state(bmc.clone()).await);
    report("bmc_status", service.bmc_status(bmc.clone()).await);
    report(
        "machine_setup_status",
        service.machine_setup_status(bmc.clone()).await,
    );
    report(
        "boot_order_status",
        service.boot_order_status(bmc.clone()).await,
    );
    report(
        "secure_boot_status",
        service.secure_boot_status(bmc.clone()).await,
    );
    report(
        "lockdown_status",
        service.lockdown_status(bmc.clone()).await,
    );
    report(
        "firmware_inventory",
        service.firmware_inventory(bmc.clone()).await,
    );

    Ok(())
}
