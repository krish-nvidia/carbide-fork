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

use nv_redfish::computer_system::ComputerSystem;
use nv_redfish::core::{EntityTypeRef, ModificationResponse, ODataETag, ODataId};
use nv_redfish::manager::Manager;
use nv_redfish::{Bmc, Error as RedfishError, Resource, ServiceRoot};
use serde::Serialize;
use serde_json::Value;

use super::{ClassifyBmcError, EtagMode, IpmiOps};
use crate::{DriverOutcome, PlatformError, PlatformIdentity};

/// Runtime-owned operation context supplied to a stateless driver.
///
/// `nv-redfish` is the only Redfish transport: drivers navigate through the
/// typed `ServiceRoot` wrappers and reach OEM resources through the same
/// authenticated `Bmc`. The ComputerSystem and Manager that exploration
/// selected are resolved once here, and PATCH requests go through
/// [`OpCx::patch`] so the BMC's [`EtagMode`] is applied in one place.
pub struct OpCx<'a, B: Bmc> {
    bmc: &'a B,
    service_root: &'a ServiceRoot<B>,
    identity: &'a PlatformIdentity,
    etag_mode: EtagMode,
    classify: fn(B::Error) -> PlatformError,
    system: Option<ComputerSystem<B>>,
    manager: Option<Manager<B>>,
    ipmi: Option<&'a dyn IpmiOps>,
}

impl<'a, B: Bmc> OpCx<'a, B> {
    /// Creates a context, resolving the exploration-selected system and manager.
    ///
    /// Fails when the identity names a resource the BMC no longer lists.
    pub async fn new(
        bmc: &'a B,
        service_root: &'a ServiceRoot<B>,
        identity: &'a PlatformIdentity,
        etag_mode: EtagMode,
    ) -> Result<Self, PlatformError>
    where
        B::Error: ClassifyBmcError,
    {
        let mut context = Self {
            bmc,
            service_root,
            identity,
            etag_mode,
            classify: <B::Error as ClassifyBmcError>::classify,
            system: None,
            manager: None,
            ipmi: None,
        };
        if let Some(system) = &identity.system {
            let systems = service_root
                .systems()
                .await
                .map_err(|error| context.map_redfish_error(error))?
                .ok_or(PlatformError::Unsupported)?
                .members()
                .await
                .map_err(|error| context.map_redfish_error(error))?;
            context.system = Some(find_selected(systems, &system.id, "ComputerSystem")?);
        }
        if let Some(manager) = &identity.manager {
            let managers = service_root
                .managers()
                .await
                .map_err(|error| context.map_redfish_error(error))?
                .ok_or(PlatformError::Unsupported)?
                .members()
                .await
                .map_err(|error| context.map_redfish_error(error))?;
            context.manager = Some(find_selected(managers, &manager.id, "Manager")?);
        }
        Ok(context)
    }

    /// Attaches IPMI operations for drivers that need them.
    pub fn with_ipmi(mut self, ipmi: &'a dyn IpmiOps) -> Self {
        self.ipmi = Some(ipmi);
        self
    }

    /// Returns the authenticated transport used to construct `service_root`.
    pub const fn bmc(&self) -> &'a B {
        self.bmc
    }

    /// Returns the service root created from `bmc`.
    pub const fn service_root(&self) -> &'a ServiceRoot<B> {
        self.service_root
    }

    /// Returns the exploration-selected identity.
    pub const fn identity(&self) -> &'a PlatformIdentity {
        self.identity
    }

    /// Returns the ComputerSystem exploration selected; `Unsupported` when the
    /// BMC manages no system (power shelves, switches).
    pub fn system(&self) -> Result<&ComputerSystem<B>, PlatformError> {
        self.system.as_ref().ok_or(PlatformError::Unsupported)
    }

    /// Returns the Manager linked to the selected system.
    pub fn manager(&self) -> Result<&Manager<B>, PlatformError> {
        self.manager.as_ref().ok_or(PlatformError::Unsupported)
    }

    /// Returns the `If-Match` convention of this BMC.
    pub const fn etag_mode(&self) -> EtagMode {
        self.etag_mode
    }

    /// Returns IPMI operations when the runtime attached them.
    pub fn ipmi(&self) -> Option<&'a dyn IpmiOps> {
        self.ipmi
    }

    /// Maps a transport failure into the platform error vocabulary.
    pub fn map_bmc_error(&self, error: B::Error) -> PlatformError {
        (self.classify)(error)
    }

    /// Maps an `nv-redfish` wrapper failure into the platform error vocabulary.
    pub fn map_redfish_error(&self, error: RedfishError<B>) -> PlatformError {
        PlatformError::from_redfish_with(error, self.classify)
    }

    /// Patches a fetched resource with its ETag under the BMC's [`EtagMode`].
    pub async fn patch<R, T>(&self, resource: &R, body: &T) -> Result<DriverOutcome, PlatformError>
    where
        R: EntityTypeRef,
        T: Serialize + Send + Sync,
    {
        self.patch_id(resource.odata_id(), resource.etag(), body)
            .await
            .map(DriverOutcome::from)
    }

    /// Patches `id` and returns the raw response for callers that must
    /// distinguish an entity from a task.
    ///
    /// `etag` is the resource's reported ETag; `None` is for resources that
    /// were never fetched, in which case the transport sends `If-Match: *`.
    pub async fn patch_id<T>(
        &self,
        id: &ODataId,
        etag: Option<&ODataETag>,
        body: &T,
    ) -> Result<ModificationResponse<Value>, PlatformError>
    where
        T: Serialize + Send + Sync,
    {
        let wildcard;
        let etag = match self.etag_mode {
            EtagMode::Resource => etag,
            EtagMode::Wildcard => {
                wildcard = ODataETag::from("*".to_string());
                Some(&wildcard)
            }
        };
        self.bmc
            .update::<_, Value>(id, etag, body)
            .await
            .map_err(|error| self.map_bmc_error(error))
    }

    /// Posts `body` to `id`, for actions and collection inserts.
    pub async fn post<T>(&self, id: &ODataId, body: &T) -> Result<DriverOutcome, PlatformError>
    where
        T: Serialize + Send + Sync,
    {
        self.post_response(id, body).await.map(DriverOutcome::from)
    }

    /// Posts `body` to `id` and returns the raw response, for callers that
    /// interpret the task location themselves.
    pub async fn post_response<T>(
        &self,
        id: &ODataId,
        body: &T,
    ) -> Result<ModificationResponse<Value>, PlatformError>
    where
        T: Serialize + Send + Sync,
    {
        self.bmc
            .create::<_, Value>(id, body)
            .await
            .map_err(|error| self.map_bmc_error(error))
    }
}

fn find_selected<R: Resource>(members: Vec<R>, id: &str, kind: &str) -> Result<R, PlatformError> {
    if id.is_empty() {
        return Err(PlatformError::InvalidResponse {
            message: format!("platform identity names an empty {kind} id"),
        });
    }
    members
        .into_iter()
        .find(|member| member.id().into_inner() == id)
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("exploration-selected {kind} {id} is no longer available"),
        })
}
