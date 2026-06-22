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

//! The shared Redfish error.
//!
//! `RedfishError` deliberately mirrors the flat error vocabulary NICo already
//! matches on today (see `libredfish::RedfishError` and the
//! `state_handler_redfish_error` mapping in `crates/redfish`). Keeping the same
//! variants means existing controller decisions -- "`NotSupported` lockdown =>
//! treat as disabled", "client create failed => wait", "Dell 404 on get_task =>
//! check version" -- keep working after the migration. There is intentionally
//! no generalized `is_retryable()`: retry stays a controller decision expressed
//! as `StateHandlerOutcome::wait(...)`.

use std::net::SocketAddr;

/// Errors surfaced by the Redfish platform service and plugins.
#[derive(Debug, thiserror::Error)]
pub enum RedfishError {
    /// Transport-level failure reaching the BMC (connect/timeout/TLS/etc).
    #[error("network error talking to {url}: {source}")]
    Network {
        /// Request URL that failed.
        url: String,
        /// Underlying transport error rendered as a string.
        #[source]
        source: BoxError,
    },

    /// The BMC returned a non-success HTTP status.
    #[error("HTTP {status_code} from {url}: {response_body}")]
    HttpStatus {
        /// Request URL.
        url: String,
        /// HTTP status code returned.
        status_code: u16,
        /// Response body (may be truncated/sanitized by the caller).
        response_body: String,
    },

    /// A response body could not be deserialized.
    #[error("failed to deserialize response from {url}: {source}")]
    Deserialize {
        /// Request URL.
        url: String,
        /// Underlying deserialization error.
        #[source]
        source: BoxError,
    },

    /// An expected JSON key was missing from a response.
    #[error("missing key `{key}` at {url}")]
    MissingKey {
        /// Missing key name.
        key: String,
        /// Resource URL.
        url: String,
    },

    /// A field held a value that could not be interpreted.
    #[error("invalid value for `{field}` at {url}: {detail}")]
    InvalidValue {
        /// Resource URL.
        url: String,
        /// Field name.
        field: String,
        /// Human-readable detail.
        detail: String,
    },

    /// A referenced boot option was not found.
    #[error("boot option not found: {0}")]
    MissingBootOption(String),

    /// The requested operation was unnecessary because the BMC was already in
    /// the desired state (e.g. power-on when already on; Dell may signal this
    /// with HTTP 409). Treated as success by most callers.
    #[error("operation unnecessary; already in desired state")]
    UnnecessaryOperation,

    /// The operation was blocked by an active lockdown.
    #[error("operation blocked by active lockdown")]
    Lockdown,

    /// The capability or operation is not supported on this platform. This is
    /// also what the service returns when the selected plugin does not
    /// implement a capability (the accessor returned `None`).
    #[error("not supported: {0}")]
    NotSupported(String),

    /// A referenced BMC account/user was not found.
    #[error("BMC user not found: {0}")]
    UserNotFound(String),

    /// The platform vendor could not be determined for selection.
    #[error("could not determine platform vendor")]
    MissingVendor,

    /// No registered plugin matched the BMC.
    #[error("no platform plugin matched the BMC")]
    NoMatchingPlugin,

    /// Plugin selection was ambiguous (more than one equally-specific match).
    /// Fails closed; resolve by making one plugin's `detect` more specific.
    #[error("ambiguous platform selection between plugins: {0:?}")]
    AmbiguousSelection(Vec<String>),

    /// The BMC requires a password change before it will accept other calls.
    #[error("BMC requires a password change before use")]
    PasswordChangeRequired,

    /// The BMC account collection is full.
    #[error("BMC account collection is full")]
    TooManyUsers,

    /// The host has no DPU; surfaced by host-side DPU-oriented operations.
    #[error("host has no DPU")]
    NoDpu,

    /// Credentials for the BMC could not be resolved.
    #[error("could not resolve credentials for BMC{}: {detail}",
        .address.map(|a| format!(" {a}")).unwrap_or_default())]
    MissingCredentials {
        /// BMC address, when known.
        address: Option<SocketAddr>,
        /// Human-readable detail.
        detail: String,
    },

    /// Catch-all for anything not covered above.
    #[error("redfish error: {0}")]
    Generic(String),
}

/// Boxed error used as a `#[source]` for transport/deserialization failures.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

impl RedfishError {
    /// Convenience constructor for the catch-all variant.
    pub fn generic(msg: impl Into<String>) -> Self {
        Self::Generic(msg.into())
    }

    /// Convenience constructor for the not-supported variant.
    pub fn not_supported(what: impl Into<String>) -> Self {
        Self::NotSupported(what.into())
    }

    /// Whether this error represents an HTTP 401/403 (auth) failure. Mirrors
    /// `libredfish::RedfishError::is_unauthorized` for callers that branch on
    /// it today.
    pub fn is_unauthorized(&self) -> bool {
        matches!(
            self,
            Self::HttpStatus {
                status_code: 401 | 403,
                ..
            }
        )
    }

    /// Whether this error represents an HTTP 404. Mirrors
    /// `libredfish::RedfishError::not_found`, used for firmware/task fallbacks.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::HttpStatus {
                status_code: 404,
                ..
            }
        )
    }
}
