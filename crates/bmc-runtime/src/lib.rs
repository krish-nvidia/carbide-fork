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

//! Runtime-owned BMC endpoint wiring, identity-based driver selection, and
//! outcome execution.
//!
//! [`Executor`] polls BMC tasks and jobs and performs the follow-up actions a
//! driver requests; controllers only see the actions that need them.

mod connection;
mod credentials;
mod endpoint;
mod error;
mod execute;
mod ipmi;
mod selection;
mod table;

pub use connection::{AuthRetryError, ConnectionManager};
pub use credentials::{
    CredentialLease, CredentialRequest, CredentialRequestError, RuntimeAuthMode,
    RuntimeCredentialProvider,
};
pub use endpoint::{BmcRef, BmcRefError, ConnectedBmc};
pub use error::ConnectError;
pub use execute::{ExecuteError, Executor, Progress};
pub use ipmi::EndpointIpmiOps;
pub use selection::{
    FirmwareVersionRange, FirmwareVersionRangeError, IdentityField, IdentityMatcher, MatchPattern,
    MatchedRule, Precedence, ResolvedSelection, RuleSet, RuleSetError, RuleSetHash, SelectionError,
    SelectionRule,
};
pub use table::{AnyDriver, DriverTable, DriverTableError};
