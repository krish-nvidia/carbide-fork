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

//! The high-level, JSON-shaped Redfish operations façade handed to plugins.
//!
//! Plugins never touch the transport (`nv-redfish`) directly. They issue
//! `GET`/`PATCH`/`POST`/`DELETE` against Redfish paths and work with
//! `serde_json::Value`, which mirrors exactly what an operator (or the
//! plugin-generation skill) sees when curling the BMC. The runtime implements
//! [`RedfishOps`] over `nv-redfish` so all transport, ETag, and action plumbing
//! lives in one place.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::RedfishError;
use crate::model::PlatformIdentity;

/// High-level Redfish operations available to a plugin capability. All paths
/// are Redfish resource paths (e.g. `/redfish/v1/Systems/1`); the runtime
/// resolves them against the BMC.
#[async_trait]
pub trait RedfishOps: Send + Sync {
    /// GET a resource as JSON.
    async fn get(&self, path: &str) -> Result<Value, RedfishError>;

    /// PATCH a resource with a JSON body. The runtime sends `If-Match: *`
    /// (last-writer-wins); there is no optimistic-concurrency ETag round-trip,
    /// which matches NICo's single-writer model and the vendors that require a
    /// wildcard `If-Match`.
    async fn patch(&self, path: &str, body: Value) -> Result<(), RedfishError>;

    /// POST a Redfish action (e.g. `.../Actions/ComputerSystem.Reset`) with a
    /// JSON body; returns any response body (often empty).
    async fn post_action(&self, path: &str, body: Value) -> Result<Value, RedfishError>;

    /// POST to a collection to create a member; returns the created resource
    /// body when provided.
    async fn create(&self, path: &str, body: Value) -> Result<Value, RedfishError>;

    /// DELETE a resource.
    async fn delete(&self, path: &str) -> Result<(), RedfishError>;

    /// Canonical first `ComputerSystem` resource path (e.g.
    /// `/redfish/v1/Systems/1`). The runtime applies the same canonical-system
    /// selection used everywhere else.
    async fn system_path(&self) -> Result<String, RedfishError>;

    /// Canonical first `Manager` resource path (e.g. `/redfish/v1/Managers/1`).
    async fn manager_path(&self) -> Result<String, RedfishError>;
}

/// Context passed to every plugin capability call. Exposes the Redfish ops
/// façade and the selection evidence; deliberately exposes no transport,
/// database, controller state, or secret internals.
pub struct PlatformExecutionContext<'a> {
    ops: &'a dyn RedfishOps,
    identity: &'a PlatformIdentity,
}

impl<'a> PlatformExecutionContext<'a> {
    /// Construct a context (used by the runtime).
    pub fn new(ops: &'a dyn RedfishOps, identity: &'a PlatformIdentity) -> Self {
        Self { ops, identity }
    }

    /// The Redfish operations façade.
    pub fn ops(&self) -> &dyn RedfishOps {
        self.ops
    }

    /// The identity evidence collected during selection.
    pub fn identity(&self) -> &PlatformIdentity {
        self.identity
    }
}
