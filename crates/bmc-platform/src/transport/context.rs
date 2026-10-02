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
use nv_redfish::core::Action;
use nv_redfish::manager::Manager;
use nv_redfish::{Bmc, Error as RedfishError, ServiceRoot};
use serde::{Deserialize, Serialize};

use super::{ClassifyBmcError, EtagMode, IpmiOps};
use crate::{DriverOutcome, PlatformError, PlatformIdentity};

/// Runtime-owned operation context supplied to a stateless driver.
///
/// `nv-redfish` is the only Redfish transport: drivers navigate through the
/// typed `ServiceRoot` wrappers and reach OEM resources through the same
/// authenticated `Bmc`. The ComputerSystem and Manager that exploration
/// selected are resolved once here.
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
            context.system = Some(find_selected(
                systems,
                |member| member.raw().id.clone(),
                &system.id,
                "ComputerSystem",
            )?);
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
            context.manager = Some(find_selected(
                managers,
                |member| member.raw().id.clone(),
                &manager.id,
                "Manager",
            )?);
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

    /// Runs an advertised Redfish action with `params`.
    ///
    /// The transport resolves the action target the same way `nv-redfish`
    /// does for its typed actions, so drivers never rebuild standard URIs.
    pub async fn action<T, R>(
        &self,
        action: &Action<T, R>,
        params: &T,
    ) -> Result<DriverOutcome, PlatformError>
    where
        T: Serialize + Send + Sync,
        R: for<'de> Deserialize<'de> + Send + Sync,
    {
        action
            .run(self.bmc, params)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| self.map_bmc_error(error))
    }
}

fn find_selected<R>(
    members: Vec<R>,
    member_id: impl Fn(&R) -> String,
    id: &str,
    kind: &str,
) -> Result<R, PlatformError> {
    if id.is_empty() {
        return Err(PlatformError::InvalidResponse {
            message: format!("platform identity names an empty {kind} id"),
        });
    }
    members
        .into_iter()
        .find(|member| member_id(member) == id)
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("exploration-selected {kind} {id} is no longer available"),
        })
}
