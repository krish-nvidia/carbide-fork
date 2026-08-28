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

use bmc_platform::{AuthError, IpmiOps, PlatformError};
use carbide_redfish::nv_redfish::{
    BmcError, Error as RedfishError, NvRedfishClientPool, RedfishBmc,
};
use thiserror::Error;

use crate::{
    BmcRef, ConnectError, ConnectedBmc, CredentialLease, CredentialRequest, RuntimeAuthMode,
    RuntimeCredentialProvider, map_redfish_error,
};

/// Boxed capability-operation future accepted by [`ConnectionManager::with_auth_retry`].
pub type PlatformOperationFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, PlatformError>> + Send + 'a>>;

/// A pooled, authenticated BMC connection retaining its transient lease.
///
/// This type intentionally implements neither serde nor `Debug`.
pub struct AuthenticatedBmc {
    request: CredentialRequest,
    lease: CredentialLease,
    connected: ConnectedBmc<RedfishBmc>,
}

impl AuthenticatedBmc {
    /// Returns the live BMC handle and its persisted driver map.
    pub const fn connected(&self) -> &ConnectedBmc<RedfishBmc> {
        &self.connected
    }

    /// Returns the secret-free credential request metadata.
    pub const fn credential_request(&self) -> &CredentialRequest {
        &self.request
    }
}

/// Constructs and refreshes concrete pooled Redfish connections.
pub struct ConnectionManager {
    pool: Arc<NvRedfishClientPool>,
    credentials: Arc<dyn RuntimeCredentialProvider>,
    auth_mode: RuntimeAuthMode,
}

impl ConnectionManager {
    /// Creates a manager from injected pool, credential, rule, and driver services.
    pub const fn new(
        pool: Arc<NvRedfishClientPool>,
        credentials: Arc<dyn RuntimeCredentialProvider>,
    ) -> Self {
        Self {
            pool,
            credentials,
            auth_mode: RuntimeAuthMode::Basic,
        }
    }

    /// Selects the runtime authentication mechanism. Basic auth is the default.
    pub const fn with_auth_mode(mut self, auth_mode: RuntimeAuthMode) -> Self {
        self.auth_mode = auth_mode;
        self
    }

    /// Acquires credentials and builds a pooled connection.
    pub async fn connect(
        &self,
        endpoint: BmcRef,
        caller_identity: String,
        ipmi: Option<Arc<dyn IpmiOps>>,
    ) -> Result<AuthenticatedBmc, ConnectError> {
        let request = CredentialRequest::new(
            caller_identity,
            endpoint.credential_key().clone(),
            endpoint.address(),
            self.auth_mode,
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
            request,
            lease,
            connected,
        })
    }

    /// Evicts every pooled root for the BMC, refreshes credentials, and rebuilds.
    pub async fn refresh(&self, authenticated: &mut AuthenticatedBmc) -> Result<(), ConnectError> {
        self.pool
            .invalidate_service_roots_for_bmc(authenticated.request.bmc_address());
        let lease = self
            .credentials
            .refresh(&authenticated.request)
            .await
            .map_err(ConnectError::Credentials)?;
        let service_root = self
            .service_root(authenticated.request.bmc_address(), &lease)
            .await?;
        authenticated.lease = lease;
        authenticated.connected.replace_service_root(service_root);
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
        F: for<'a> Fn(&'a ConnectedBmc<RedfishBmc>) -> PlatformOperationFuture<'a, T>,
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
        endpoint: &BmcRef,
        lease: &CredentialLease,
        ipmi: Option<Arc<dyn IpmiOps>>,
    ) -> Result<ConnectedBmc<RedfishBmc>, ConnectError> {
        let root = self.service_root(endpoint.address(), lease).await?;
        Ok(ConnectedBmc::new(endpoint.clone(), root, ipmi))
    }

    async fn service_root(
        &self,
        address: std::net::SocketAddr,
        lease: &CredentialLease,
    ) -> Result<Arc<nv_redfish::ServiceRoot<RedfishBmc>>, ConnectError> {
        self.pool
            .service_root_with_bmc_credentials(address, lease.credentials().clone())
            .await
            .map_err(|error| ConnectError::Transport(map_concrete_error(error)))
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
    Operation(PlatformError),
}

const fn is_auth_error(error: &PlatformError) -> bool {
    matches!(
        error,
        PlatformError::Auth(AuthError::InvalidCredentials | AuthError::InsufficientPrivilege)
    )
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

    use super::is_auth_error;
    use bmc_platform::{AuthError, PlatformError};

    #[test]
    fn only_unauthorized_and_forbidden_trigger_one_shot_refresh() {
        value_scenarios!(run = |error: PlatformError| is_auth_error(&error);
            "refresh once" {
                PlatformError::Auth(AuthError::InvalidCredentials) => true,
                PlatformError::Auth(AuthError::InsufficientPrivilege) => true,
            }
            "return directly" {
                PlatformError::Unsupported => false,
                PlatformError::Unreachable => false,
                PlatformError::LockedDown => false,
            }
        );
    }
}
