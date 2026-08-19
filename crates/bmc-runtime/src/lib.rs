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

//! Runtime-owned BMC endpoint wiring, identity projection, and driver selection.
//!
//! This crate does not poll Redfish tasks or execute controller actions.
//! Controllers retain responsibility for interpreting [`bmc_platform::DriverOutcome`].

mod connection;
mod credentials;
mod endpoint;
mod error;
mod identity;
mod ipmi;
mod outcome;
mod registry;
mod selection;

pub use connection::{AuthRetryError, AuthenticatedBmc, ConnectionManager, RedfishOperationFuture};
pub use credentials::{
    CredentialLease, CredentialRequest, CredentialRequestError, RuntimeCredentialProvider,
};
pub use endpoint::{BmcEndpoint, BmcRef, BmcRefError, ConnectedBmc};
pub use error::{ConnectError, IdentityProjectionError, map_redfish_error};
pub use identity::project_platform_identity;
pub use ipmi::EndpointIpmiOps;
pub use outcome::driver_outcome;
pub use registry::{DispatchError, DriverRegistry, DriverSet, RegistryError};
pub use selection::{
    FirmwareVersionRange, FirmwareVersionRangeError, IdentityField, IdentityMatcher, MatchPattern,
    Precedence, ResolvedSelection, RuleSet, RuleSetError, RuleSetHash, SelectionError,
    SelectionRule,
};
