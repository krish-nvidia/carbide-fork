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

//! Registration glue shared between the plugins crate and the runtime.
//!
//! The plugins crate calls `register_all(&mut dyn PluginRegistrar)`; the runtime
//! provides a registrar (its registry builder). Defining the trait here keeps
//! the plugins crate free of any dependency on the runtime.

use std::sync::Arc;

use crate::plugin::PlatformPlugin;

/// A sink that accepts plugins during explicit registration.
pub trait PluginRegistrar {
    /// Register one plugin.
    fn register(&mut self, plugin: Arc<dyn PlatformPlugin>);
}
