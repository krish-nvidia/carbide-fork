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

//! Bridge from the Redfish platform contract's [`RedfishError`] into the
//! state-controller error type.
//!
//! This mirrors [`crate::libredfish::error::state_handler_redfish_error`] so a
//! caller migrating off `libredfish` onto `RedfishPlatformService` keeps the
//! exact same `ExternalServiceError` shape and metric labels it has today.

use carbide_redfish_platform_api::RedfishError;
use state_controller::state_handler::{ExternalServiceError, StateHandlerError};

/// Map a platform [`RedfishError`] into a [`StateHandlerError`], tagging it with
/// the operation name and the existing `redfish_*` metric label.
pub fn state_handler_platform_error(
    operation: &'static str,
    error: RedfishError,
) -> StateHandlerError {
    ExternalServiceError::with_source(
        "redfish",
        operation,
        error.to_string(),
        platform_operation_metric_label(operation),
        error,
    )
    .into()
}

fn platform_operation_metric_label(operation: &'static str) -> &'static str {
    match operation {
        "restart" | "set_power" | "reset_bmc" => "redfish_restart_error",
        "set_host_lockdown" | "set_bmc_lockdown" | "lockdown_status" => "redfish_lockdown_error",
        _ => "redfish_other_error",
    }
}
