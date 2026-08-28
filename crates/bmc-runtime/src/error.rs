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

use bmc_platform::PlatformError;
use nv_redfish::{Bmc, Error as RedfishError};
use thiserror::Error;

use crate::CredentialRequestError;

/// Failure while authenticating and connecting to a BMC.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ConnectError {
    /// Credential request metadata is invalid.
    #[error(transparent)]
    CredentialRequest(#[from] CredentialRequestError),
    /// Credential acquisition or refresh failed.
    #[error("BMC credential provider failed: {0}")]
    Credentials(PlatformError),
    /// Concrete Redfish transport or authentication failed.
    #[error("BMC connection failed: {0}")]
    Transport(PlatformError),
}

/// Maps errors shared by `nv-redfish` wrappers into platform errors.
///
/// The caller classifies `B::Error`, because only the concrete transport knows
/// whether its error is authentication, reachability, or a BMC response.
pub fn map_redfish_error<B, F>(error: RedfishError<B>, map_bmc: F) -> PlatformError
where
    B: Bmc,
    F: FnOnce(B::Error) -> PlatformError,
{
    match error {
        RedfishError::Bmc(error) => map_bmc(error),
        RedfishError::AccountSlotNotAvailable => PlatformError::TooManyUsers,
        RedfishError::ActionNotAvailable => PlatformError::Unsupported,
        RedfishError::Json(error) => PlatformError::InvalidResponse {
            message: error.to_string(),
        },
        other => PlatformError::InvalidResponse {
            message: other.to_string(),
        },
    }
}
