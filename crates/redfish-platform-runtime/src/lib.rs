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

//! Runtime for the Redfish platform contract.
//!
//! Owns credential resolution, per-call `nv-redfish` session creation,
//! deterministic plugin selection, and dispatch into the selected plugin's
//! capability implementations. State controllers depend only on
//! `carbide-redfish-platform-api`; the binary constructs a runtime via
//! [`build_runtime`] and injects it as `Arc<dyn RedfishPlatformService>`.
//!
//! Credentials are decoupled behind [`BmcCredentialProvider`] so the runtime
//! does not hard-depend on the secrets crate; the binary adapts its existing
//! `carbide_secrets::CredentialReader` (keyed by BMC MAC, as today).

use std::sync::Arc;

use async_trait::async_trait;
use carbide_redfish_platform_api::error::RedfishError;
use carbide_redfish_platform_api::model::{BmcRef, PlatformIdentity, PluginId};

pub mod ops_impl;
pub mod registry;
pub mod runtime;

pub use registry::{PlatformRegistry, PlatformRegistryBuilder};
pub use runtime::RedfishPlatformRuntime;

/// Select the platform plugin id for an identity using the first-party plugin
/// bundle and the exact selection the runtime uses at call time.
///
/// Site exploration calls this to cache a stable, registry-backed plugin id on
/// the explored endpoint (later passed as a [`BmcRef::platform_hint`]). Because
/// it runs the same `register_all` + `PlatformRegistry::select`, the result is
/// always a real plugin id the runtime would pick, or `None` when nothing
/// matches (callers then fall back to live identification).
pub fn select_plugin_id(identity: &PlatformIdentity) -> Option<PluginId> {
    let mut builder = PlatformRegistryBuilder::new();
    carbide_redfish_platform_plugins::register_all(&mut builder);
    builder
        .build()
        .select(identity)
        .ok()
        .map(|(plugin, _)| plugin.metadata().id.clone())
}

// The credential DTO lives in the API crate (callers attach it to `BmcRef`);
// re-exported here for the provider implementations the binary writes.
pub use carbide_redfish_platform_api::model::BmcCredentials;

/// Resolves BMC credentials for a [`BmcRef`]. Implemented by the binary,
/// typically by adapting `carbide_secrets::CredentialReader` keyed by the BMC
/// MAC address (`CredentialKey::BmcCredentials { BmcRoot { bmc_mac_address } }`).
#[async_trait]
pub trait BmcCredentialProvider: Send + Sync {
    /// Return the login credentials for the given BMC.
    async fn credentials_for(&self, bmc: &BmcRef) -> Result<BmcCredentials, RedfishError>;
}

/// How the runtime authenticates to a BMC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    /// HTTP Basic auth per request (the default, matching NICo today).
    #[default]
    BasicAuth,
    /// Establish a Redfish `SessionService` token, use it for the connection's
    /// requests, and best-effort delete the session when the connection drops.
    /// Opt-in; not used by default.
    Session,
}

/// Build a ready-to-use platform service with the first-party plugin bundle
/// registered, using HTTP Basic auth.
pub fn build_runtime(
    credentials: Arc<dyn BmcCredentialProvider>,
) -> Arc<dyn carbide_redfish_platform_api::service::RedfishPlatformService> {
    build_runtime_with_auth(credentials, AuthMode::default())
}

/// Build a ready-to-use platform service with an explicit authentication mode.
pub fn build_runtime_with_auth(
    credentials: Arc<dyn BmcCredentialProvider>,
    auth_mode: AuthMode,
) -> Arc<dyn carbide_redfish_platform_api::service::RedfishPlatformService> {
    let mut builder = PlatformRegistryBuilder::new();
    carbide_redfish_platform_plugins::register_all(&mut builder);
    Arc::new(RedfishPlatformRuntime::new(
        builder.build(),
        credentials,
        auth_mode,
    ))
}
