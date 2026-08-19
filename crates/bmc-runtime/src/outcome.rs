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

use bmc_platform::{DriverOutcome, OperationReference};
use nv_redfish::core::ModificationResponse;

/// Converts a Redfish mutation response into the shared driver outcome.
///
/// Entity bodies are intentionally discarded at this boundary. Async task
/// references are returned to the controller, which owns polling and actions.
pub fn driver_outcome<T>(response: ModificationResponse<T>) -> DriverOutcome {
    match response {
        ModificationResponse::Entity(_) | ModificationResponse::Empty => DriverOutcome::complete(),
        ModificationResponse::Task(task) => {
            DriverOutcome::Accepted(OperationReference::RedfishTask {
                uri: task.location.0,
                retry_after_seconds: task.retry_after.map(|duration| duration.as_secs()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use carbide_test_support::value_scenarios;
    use nv_redfish::core::{AsyncTask, AsyncTaskLocation, ODataId};

    use super::*;

    #[test]
    fn synchronous_mutations_are_complete() {
        value_scenarios!(run = driver_outcome;
            "synchronous success" {
                ModificationResponse::Entity(()) => DriverOutcome::complete(),
                ModificationResponse::Empty => DriverOutcome::complete(),
            }
        );
    }

    #[test]
    fn async_mutation_preserves_controller_polling_reference() {
        let uri = ODataId::from("/redfish/v1/TaskService/Tasks/42".to_string());
        let outcome = driver_outcome::<()>(ModificationResponse::Task(AsyncTask {
            location: AsyncTaskLocation(uri.clone()),
            retry_after: Some(Duration::from_secs(7)),
        }));

        assert_eq!(
            outcome,
            DriverOutcome::Accepted(OperationReference::RedfishTask {
                uri,
                retry_after_seconds: Some(7),
            })
        );
    }
}
