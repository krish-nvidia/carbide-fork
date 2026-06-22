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

//! First-party Redfish platform plugins for NICo plus the shared standard
//! providers.
//!
//! Vendors are modules in this single crate (mirroring how `libredfish` keeps
//! `dell.rs`, `hpe.rs`, ... together). [`register_all`] performs deterministic,
//! explicit registration into the runtime's registry.
//!
//! New vendor plugins (including ones produced by the plugin-generation skill)
//! are added as a module here, constructed in [`all_plugins`].

use std::sync::Arc;

use carbide_redfish_platform_api::PluginRegistrar;
use carbide_redfish_platform_api::plugin::PlatformPlugin;

pub mod hpe;
pub mod providers;
pub mod standard;

/// Build every first-party plugin, ordered standard-first (selection picks the
/// most specific match regardless of order).
pub fn all_plugins() -> Vec<Arc<dyn PlatformPlugin>> {
    vec![
        Arc::new(standard::StandardPlugin::new()),
        Arc::new(hpe::HpeIloPlugin::new()),
    ]
}

/// Register every first-party plugin into the given registrar.
pub fn register_all(registrar: &mut dyn PluginRegistrar) {
    for plugin in all_plugins() {
        registrar.register(plugin);
    }
}
