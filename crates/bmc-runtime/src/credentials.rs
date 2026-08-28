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

use std::fmt;
use std::net::SocketAddr;

use async_trait::async_trait;
use bmc_platform::PlatformError;
use carbide_secrets::credentials::{BmcCredentialType, CredentialKey};
use mac_address::MacAddress;
use nv_redfish::bmc_http::BmcCredentials;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Authentication mechanism used for runtime Redfish connections.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAuthMode {
    /// Read the stored BMC username/password directly.
    #[default]
    Basic,
    /// Ask the existing session manager to issue an `X-Auth-Token`.
    Session,
}

/// Secret-free metadata required for a runtime credential request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialRequest {
    caller_identity: String,
    credential_key: CredentialKey,
    bmc_address: SocketAddr,
    auth_mode: RuntimeAuthMode,
}

impl CredentialRequest {
    /// Creates request metadata for one caller and BMC endpoint.
    pub fn new(
        caller_identity: String,
        credential_key: CredentialKey,
        bmc_address: SocketAddr,
        auth_mode: RuntimeAuthMode,
    ) -> Result<Self, CredentialRequestError> {
        if caller_identity.trim().is_empty() {
            return Err(CredentialRequestError::EmptyCallerIdentity);
        }
        if !matches!(
            credential_key,
            CredentialKey::BmcCredentials {
                credential_type: BmcCredentialType::BmcRoot { .. },
            }
        ) {
            return Err(CredentialRequestError::UnsupportedCredentialKey);
        }
        Ok(Self {
            caller_identity,
            credential_key,
            bmc_address,
            auth_mode,
        })
    }

    /// Returns the authenticated service or controller requesting credentials.
    pub fn caller_identity(&self) -> &str {
        &self.caller_identity
    }

    /// Returns the BMC MAC address used for secret lookup.
    pub const fn bmc_mac_address(&self) -> MacAddress {
        match &self.credential_key {
            CredentialKey::BmcCredentials {
                credential_type: BmcCredentialType::BmcRoot { bmc_mac_address },
            } => *bmc_mac_address,
            _ => unreachable!(),
        }
    }

    /// Returns the canonical key identifying the credentials requested.
    pub const fn credential_key(&self) -> &CredentialKey {
        &self.credential_key
    }

    /// Returns the concrete BMC socket address used by the client pool.
    pub const fn bmc_address(&self) -> SocketAddr {
        self.bmc_address
    }

    /// Returns the requested authentication mechanism.
    pub const fn auth_mode(&self) -> RuntimeAuthMode {
        self.auth_mode
    }
}

/// Error returned for empty caller identity metadata.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CredentialRequestError {
    #[error("credential request caller identity must not be empty")]
    EmptyCallerIdentity,
    #[error("runtime credentials require a per-BMC root credential key")]
    UnsupportedCredentialKey,
}

/// A short-lived, non-serializable lease containing credentials for one BMC.
///
/// The lease deliberately exposes no secret-bearing `Debug`, `Display`, or
/// serde representation.
#[derive(Clone)]
pub struct CredentialLease {
    credentials: BmcCredentials,
}

impl CredentialLease {
    /// Creates a transient credential lease.
    pub const fn new(credentials: BmcCredentials) -> Self {
        Self { credentials }
    }

    /// Borrows the credentials for constructing or refreshing a transport.
    pub const fn credentials(&self) -> &BmcCredentials {
        &self.credentials
    }
}

impl fmt::Debug for CredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialLease")
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

/// Acquires and refreshes transient credentials for runtime BMC connections.
#[async_trait]
pub trait RuntimeCredentialProvider: Send + Sync {
    /// Acquires a new credential lease for `request`.
    async fn acquire(&self, request: &CredentialRequest) -> Result<CredentialLease, PlatformError>;

    /// Refreshes credentials for `request`, returning a replacement lease.
    async fn refresh(&self, request: &CredentialRequest) -> Result<CredentialLease, PlatformError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;

    fn credential_key(mac_address: MacAddress) -> CredentialKey {
        CredentialKey::BmcCredentials {
            credential_type: BmcCredentialType::BmcRoot {
                bmc_mac_address: mac_address,
            },
        }
    }

    struct RecordingProvider {
        requests: Mutex<Vec<CredentialRequest>>,
    }

    #[async_trait]
    impl RuntimeCredentialProvider for RecordingProvider {
        async fn acquire(
            &self,
            request: &CredentialRequest,
        ) -> Result<CredentialLease, PlatformError> {
            self.requests
                .lock()
                .expect("request recorder mutex")
                .push(request.clone());
            Ok(CredentialLease::new(BmcCredentials::token(
                "first-secret".to_string(),
            )))
        }

        async fn refresh(
            &self,
            request: &CredentialRequest,
        ) -> Result<CredentialLease, PlatformError> {
            self.requests
                .lock()
                .expect("request recorder mutex")
                .push(request.clone());
            Ok(CredentialLease::new(BmcCredentials::token(
                "second-secret".to_string(),
            )))
        }
    }

    #[test]
    fn credential_lease_debug_is_fully_redacted() {
        let lease = CredentialLease::new(BmcCredentials::new(
            "operator".to_string(),
            "secret-value".to_string(),
        ));

        let debug = format!("{lease:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("operator"));
        assert!(!debug.contains("secret-value"));
    }

    #[test]
    fn runtime_auth_defaults_to_basic() {
        assert_eq!(RuntimeAuthMode::default(), RuntimeAuthMode::Basic);
    }

    #[test]
    fn credential_request_contains_explicit_secret_free_metadata() {
        let request = CredentialRequest::new(
            "machine-controller".to_string(),
            credential_key(MacAddress::new([2, 0, 0, 0, 0, 1])),
            "192.0.2.10:443".parse().expect("valid socket address"),
            RuntimeAuthMode::Basic,
        )
        .expect("caller identity is valid");

        let encoded = serde_json::to_string(&request).expect("request serializes");
        assert!(encoded.contains("machine-controller"));
        assert!(encoded.contains("192.0.2.10:443"));
        assert!(encoded.contains("02:00:00:00:00:01"));
        assert!(!encoded.contains("password"));
        assert!(!encoded.contains("token"));
    }

    #[tokio::test]
    async fn provider_acquire_and_refresh_receive_identical_request_metadata() {
        let request = CredentialRequest::new(
            "rack-controller".to_string(),
            credential_key(MacAddress::new([2, 0, 0, 0, 0, 3])),
            "192.0.2.30:443".parse().expect("valid socket address"),
            RuntimeAuthMode::Session,
        )
        .expect("caller identity is valid");
        let provider = RecordingProvider {
            requests: Mutex::new(Vec::new()),
        };

        provider.acquire(&request).await.expect("acquire succeeds");
        provider.refresh(&request).await.expect("refresh succeeds");

        let requests = provider.requests.lock().expect("request recorder mutex");
        assert_eq!(requests.len(), 2);
        for recorded in requests.iter() {
            assert_eq!(recorded.caller_identity(), request.caller_identity());
            assert_eq!(recorded.bmc_address(), request.bmc_address());
            assert_eq!(recorded.bmc_mac_address(), request.bmc_mac_address());
            assert_eq!(recorded.auth_mode(), request.auth_mode());
        }
    }
}
