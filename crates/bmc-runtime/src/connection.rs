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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use bmc_platform::{AuthError, IpmiOps, PlatformError};
use carbide_redfish::nv_redfish::{
    BmcError, Error as RedfishError, NvRedfishClientPool, RedfishBmc,
};
use nv_redfish::Error as NvRedfishError;
use thiserror::Error;

use crate::{
    BmcEndpoint, ConnectError, ConnectedBmc, CredentialLease, CredentialRequest, DriverRegistry,
    RuleSet, RuntimeCredentialProvider, map_redfish_error,
};

/// Boxed operation future accepted by [`ConnectionManager::with_auth_retry`].
pub type RedfishOperationFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RedfishError>> + Send + 'a>>;

/// A pooled, authenticated BMC connection retaining its transient lease.
///
/// This type intentionally implements neither serde nor `Debug`.
pub struct AuthenticatedBmc {
    endpoint: BmcEndpoint,
    request: CredentialRequest,
    lease: CredentialLease,
    connected: ConnectedBmc<RedfishBmc>,
    ipmi: Option<Arc<dyn IpmiOps>>,
}

impl AuthenticatedBmc {
    /// Returns the live selected BMC handle.
    pub const fn connected(&self) -> &ConnectedBmc<RedfishBmc> {
        &self.connected
    }

    /// Returns the secret-free credential request metadata.
    pub const fn credential_request(&self) -> &CredentialRequest {
        &self.request
    }

    /// Returns the lease expiration without exposing credential material.
    pub const fn credential_expires_at(&self) -> Option<SystemTime> {
        self.lease.expires_at()
    }
}

/// Constructs and refreshes concrete pooled Redfish connections.
pub struct ConnectionManager {
    pool: Arc<NvRedfishClientPool>,
    credentials: Arc<dyn RuntimeCredentialProvider>,
    rules: Arc<RuleSet>,
    registry: Arc<DriverRegistry<RedfishBmc>>,
}

impl ConnectionManager {
    /// Creates a manager from injected pool, credential, rule, and driver services.
    pub const fn new(
        pool: Arc<NvRedfishClientPool>,
        credentials: Arc<dyn RuntimeCredentialProvider>,
        rules: Arc<RuleSet>,
        registry: Arc<DriverRegistry<RedfishBmc>>,
    ) -> Self {
        Self {
            pool,
            credentials,
            rules,
            registry,
        }
    }

    /// Acquires credentials and builds a selected pooled connection.
    pub async fn connect(
        &self,
        endpoint: BmcEndpoint,
        caller_identity: String,
        ipmi: Option<Arc<dyn IpmiOps>>,
    ) -> Result<AuthenticatedBmc, ConnectError> {
        let reference = endpoint.reference();
        let request = CredentialRequest::new(
            caller_identity,
            reference.mac_address(),
            reference.address(),
        )?;
        let lease = self
            .credentials
            .acquire(&request)
            .await
            .map_err(ConnectError::Credentials)?;
        let connected = self
            .build_connected(&endpoint, &lease, ipmi.clone())
            .await?;
        Ok(AuthenticatedBmc {
            endpoint,
            request,
            lease,
            connected,
            ipmi,
        })
    }

    /// Evicts every pooled root for the BMC, refreshes credentials, and rebuilds.
    ///
    /// The prior handle remains intact if refresh or reconstruction fails.
    pub async fn refresh(&self, authenticated: &mut AuthenticatedBmc) -> Result<(), ConnectError> {
        self.pool
            .invalidate_service_roots_for_bmc(authenticated.request.bmc_address());
        let lease = self
            .credentials
            .refresh(&authenticated.request, &authenticated.lease)
            .await
            .map_err(ConnectError::Credentials)?;
        let connected = self
            .build_connected(&authenticated.endpoint, &lease, authenticated.ipmi.clone())
            .await?;
        authenticated.lease = lease;
        authenticated.connected = connected;
        Ok(())
    }

    /// Runs an operation and retries it once after a 401 or 403 refresh.
    ///
    /// Non-authentication errors and failures from the second attempt are
    /// returned without another refresh.
    pub async fn with_auth_retry<T, F>(
        &self,
        authenticated: &mut AuthenticatedBmc,
        operation: F,
    ) -> Result<T, AuthRetryError>
    where
        F: for<'a> Fn(&'a ConnectedBmc<RedfishBmc>) -> RedfishOperationFuture<'a, T>,
    {
        match operation(&authenticated.connected).await {
            Ok(value) => Ok(value),
            Err(error) if is_auth_error(&error) => {
                self.refresh(authenticated)
                    .await
                    .map_err(AuthRetryError::Refresh)?;
                operation(&authenticated.connected)
                    .await
                    .map_err(AuthRetryError::Operation)
            }
            Err(error) => Err(AuthRetryError::Operation(error)),
        }
    }

    async fn build_connected(
        &self,
        endpoint: &BmcEndpoint,
        lease: &CredentialLease,
        ipmi: Option<Arc<dyn IpmiOps>>,
    ) -> Result<ConnectedBmc<RedfishBmc>, ConnectError> {
        let root = self
            .pool
            .service_root_with_bmc_credentials(
                endpoint.reference().address(),
                lease.credentials().clone(),
            )
            .await
            .map_err(|error| ConnectError::Transport(map_concrete_error(error)))?;
        ConnectedBmc::connect(
            endpoint.clone(),
            root,
            &self.rules,
            self.registry.clone(),
            ipmi,
        )
        .await
    }
}

/// Failure from the one-shot authenticated operation API.
#[derive(Debug, Error)]
pub enum AuthRetryError {
    /// Credential refresh or connection reconstruction failed.
    #[error("BMC authentication refresh failed: {0}")]
    Refresh(ConnectError),
    /// The operation failed without a retry or after its only retry.
    #[error("BMC operation failed: {0}")]
    Operation(RedfishError),
}

fn is_auth_error(error: &RedfishError) -> bool {
    matches!(
        error,
        NvRedfishError::Bmc(BmcError::InvalidResponse { status, .. })
            if is_auth_status(status.as_u16())
    )
}

const fn is_auth_status(status: u16) -> bool {
    matches!(status, 401 | 403)
}

fn map_concrete_error(error: RedfishError) -> PlatformError {
    map_redfish_error(error, |error| match error {
        BmcError::ReqwestError(_) => PlatformError::Unreachable,
        BmcError::InvalidResponse { status, text, .. } => match status.as_u16() {
            401 => PlatformError::Auth(AuthError::InvalidCredentials),
            403 => PlatformError::Auth(AuthError::InsufficientPrivilege),
            status => PlatformError::Bmc {
                status,
                message_id: None,
                message: text,
            },
        },
        other => PlatformError::InvalidResponse {
            message: other.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use carbide_test_support::value_scenarios;

    use super::is_auth_status;

    #[test]
    fn only_unauthorized_and_forbidden_trigger_one_shot_refresh() {
        value_scenarios!(run = is_auth_status;
            "refresh once" {
                401 => true,
                403 => true,
            }
            "return directly" {
                400 => false,
                404 => false,
                500 => false,
            }
        );
    }
}
