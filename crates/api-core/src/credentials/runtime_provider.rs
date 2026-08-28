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

use async_trait::async_trait;
use bmc_platform::{AuthError, PlatformError};
use bmc_runtime::{CredentialLease, CredentialRequest, RuntimeAuthMode, RuntimeCredentialProvider};
use carbide_secrets::credentials::Credentials;
use nv_redfish::bmc_http::BmcCredentials;

use super::{BmcAuthMaterial, BmcSessionError, BmcSessionManager};

#[async_trait]
impl RuntimeCredentialProvider for BmcSessionManager {
    async fn acquire(&self, request: &CredentialRequest) -> Result<CredentialLease, PlatformError> {
        issue_runtime_credentials(self, request).await
    }

    async fn refresh(&self, request: &CredentialRequest) -> Result<CredentialLease, PlatformError> {
        issue_runtime_credentials(self, request).await
    }
}

async fn issue_runtime_credentials(
    manager: &BmcSessionManager,
    request: &CredentialRequest,
) -> Result<CredentialLease, PlatformError> {
    let material = match request.auth_mode() {
        RuntimeAuthMode::Basic => manager
            .bmc_root_credentials(request.bmc_mac_address())
            .await
            .map(BmcAuthMaterial::Basic),
        RuntimeAuthMode::Session => {
            manager
                .issue_credentials(
                    request.caller_identity(),
                    request.bmc_mac_address(),
                    request.bmc_address(),
                )
                .await
        }
    };
    material
        .map(runtime_credential_lease)
        .map_err(map_session_error)
}

fn runtime_credential_lease(material: BmcAuthMaterial) -> CredentialLease {
    let credentials = match material {
        BmcAuthMaterial::Session(session) => BmcCredentials::token(session.token),
        BmcAuthMaterial::Basic(Credentials::UsernamePassword { username, password }) => {
            BmcCredentials::new(username, password)
        }
    };
    CredentialLease::new(credentials)
}

fn map_session_error(error: BmcSessionError) -> PlatformError {
    match error {
        BmcSessionError::MissingRootCredentials(_) | BmcSessionError::CredentialStore(_) => {
            PlatformError::Auth(AuthError::CredentialsUnavailable)
        }
        BmcSessionError::NoSessionService { .. } => PlatformError::Unsupported,
        BmcSessionError::AvoidLockout {
            consecutive_unauthorized,
            last_status,
            ..
        } => PlatformError::Bmc {
            status: last_status,
            message_id: None,
            message: format!(
                "BMC session lockout breaker tripped after {consecutive_unauthorized} unauthorized responses"
            ),
        },
        BmcSessionError::Redfish { detail, .. } | BmcSessionError::Store(detail) => {
            PlatformError::InvalidResponse { message: detail }
        }
    }
}

#[cfg(test)]
mod tests {
    use mac_address::MacAddress;

    use super::*;

    #[test]
    fn basic_credentials_convert_without_serializable_runtime_state() {
        let lease =
            runtime_credential_lease(BmcAuthMaterial::Basic(Credentials::UsernamePassword {
                username: "operator".to_string(),
                password: "secret".to_string(),
            }));

        assert_eq!(
            lease.credentials(),
            &BmcCredentials::new("operator".to_string(), "secret".to_string())
        );
        let debug = format!("{lease:?}");
        assert!(!debug.contains("operator"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn session_errors_map_to_stable_platform_errors() {
        let mac = MacAddress::new([2, 0, 0, 0, 0, 1]);

        assert_eq!(
            map_session_error(BmcSessionError::MissingRootCredentials(mac)),
            PlatformError::Auth(AuthError::CredentialsUnavailable)
        );
        assert_eq!(
            map_session_error(BmcSessionError::AvoidLockout {
                bmc_mac: mac,
                consecutive_unauthorized: 3,
                last_status: 401,
            }),
            PlatformError::Bmc {
                status: 401,
                message_id: None,
                message: "BMC session lockout breaker tripped after 3 unauthorized responses"
                    .to_string(),
            }
        );
    }
}
