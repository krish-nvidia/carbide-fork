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
use std::time::SystemTime;

use async_trait::async_trait;
use bmc_platform::PlatformError;
use mac_address::MacAddress;
use nv_redfish::bmc_http::BmcCredentials;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Secret-free metadata required for a runtime credential request.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct CredentialRequest {
    caller_identity: String,
    bmc_mac_address: MacAddress,
    bmc_address: SocketAddr,
}

impl CredentialRequest {
    /// Creates request metadata for one caller and BMC endpoint.
    pub fn new(
        caller_identity: String,
        bmc_mac_address: MacAddress,
        bmc_address: SocketAddr,
    ) -> Result<Self, CredentialRequestError> {
        if caller_identity.trim().is_empty() {
            return Err(CredentialRequestError);
        }
        Ok(Self {
            caller_identity,
            bmc_mac_address,
            bmc_address,
        })
    }

    /// Returns the authenticated service or controller requesting credentials.
    pub fn caller_identity(&self) -> &str {
        &self.caller_identity
    }

    /// Returns the BMC MAC address used for secret lookup.
    pub const fn bmc_mac_address(&self) -> MacAddress {
        self.bmc_mac_address
    }

    /// Returns the concrete BMC socket address used by the client pool.
    pub const fn bmc_address(&self) -> SocketAddr {
        self.bmc_address
    }
}

/// Error returned for empty caller identity metadata.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("credential request caller identity must not be empty")]
pub struct CredentialRequestError;

/// A short-lived, non-serializable lease containing credentials for one BMC.
///
/// The lease deliberately exposes no secret-bearing `Debug`, `Display`, or
/// serde representation.
#[derive(Clone)]
pub struct CredentialLease {
    credentials: BmcCredentials,
    expires_at: Option<SystemTime>,
}

impl CredentialLease {
    /// Creates a lease with an optional provider-defined expiration time.
    pub const fn new(credentials: BmcCredentials, expires_at: Option<SystemTime>) -> Self {
        Self {
            credentials,
            expires_at,
        }
    }

    /// Borrows the credentials for constructing or refreshing a transport.
    pub const fn credentials(&self) -> &BmcCredentials {
        &self.credentials
    }

    /// Consumes the lease and returns its credentials.
    pub fn into_credentials(self) -> BmcCredentials {
        self.credentials
    }

    /// Returns the provider-defined expiration time, when one exists.
    pub const fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

impl fmt::Debug for CredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialLease")
            .field("credentials", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Acquires and refreshes transient credentials for runtime BMC connections.
#[async_trait]
pub trait RuntimeCredentialProvider: Send + Sync {
    /// Acquires a new credential lease for `request`.
    async fn acquire(&self, request: &CredentialRequest) -> Result<CredentialLease, PlatformError>;

    /// Refreshes `current` for `request`, returning a replacement lease.
    async fn refresh(
        &self,
        request: &CredentialRequest,
        current: &CredentialLease,
    ) -> Result<CredentialLease, PlatformError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;

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
            Ok(CredentialLease::new(
                BmcCredentials::token("first-secret".to_string()),
                None,
            ))
        }

        async fn refresh(
            &self,
            request: &CredentialRequest,
            _current: &CredentialLease,
        ) -> Result<CredentialLease, PlatformError> {
            self.requests
                .lock()
                .expect("request recorder mutex")
                .push(request.clone());
            Ok(CredentialLease::new(
                BmcCredentials::token("second-secret".to_string()),
                None,
            ))
        }
    }

    #[test]
    fn credential_lease_debug_is_fully_redacted() {
        let lease = CredentialLease::new(
            BmcCredentials::new("operator".to_string(), "secret-value".to_string()),
            None,
        );

        let debug = format!("{lease:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("operator"));
        assert!(!debug.contains("secret-value"));
    }

    #[test]
    fn credential_request_contains_explicit_secret_free_metadata() {
        let request = CredentialRequest::new(
            "machine-controller".to_string(),
            MacAddress::new([2, 0, 0, 0, 0, 1]),
            "192.0.2.10:443".parse().expect("valid socket address"),
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
            MacAddress::new([2, 0, 0, 0, 0, 3]),
            "192.0.2.30:443".parse().expect("valid socket address"),
        )
        .expect("caller identity is valid");
        let provider = RecordingProvider {
            requests: Mutex::new(Vec::new()),
        };

        let lease = provider.acquire(&request).await.expect("acquire succeeds");
        provider
            .refresh(&request, &lease)
            .await
            .expect("refresh succeeds");

        assert_eq!(
            provider
                .requests
                .lock()
                .expect("request recorder mutex")
                .as_slice(),
            &[request.clone(), request]
        );
    }
}
