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

use carbide_test_support::value_scenarios;
use serde_json::{Value, json};

use super::*;

fn with_envelope(uri: &str, id: &str, fields: Value) -> Value {
    let mut body = json!({"@odata.id": uri, "Id": id, "Name": id});
    body.as_object_mut()
        .expect("body is an object")
        .extend(fields.as_object().expect("fields are an object").clone());
    body
}

fn task(fields: Value) -> Task {
    serde_json::from_value(with_envelope(
        "/redfish/v1/TaskService/Tasks/7",
        "7",
        fields,
    ))
    .expect("task body")
}

fn job(fields: Value) -> Job {
    serde_json::from_value(with_envelope("/redfish/v1/JobService/Jobs/3", "3", fields))
        .expect("job body")
}

fn dell_job(fields: Value) -> DellJob {
    serde_json::from_value(with_envelope(
        "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/Jobs/JID_1",
        "JID_1",
        fields,
    ))
    .expect("Dell job body")
}

fn failed_with(state: &str, message: &str) -> Result<OperationStatus, ()> {
    Ok(OperationStatus::Failed {
        state: state.to_string(),
        message: Some(message.to_string()),
    })
}

fn needs_intervention(state: &str) -> Result<OperationStatus, ()> {
    Ok(OperationStatus::NeedsIntervention {
        state: state.to_string(),
    })
}

#[test]
fn task_states_classify_by_whether_the_work_can_still_finish() {
    value_scenarios!(run = |fields: Value| task_status(&task(fields)).map_err(drop);
        "still running" {
            json!({}) => Ok(OperationStatus::Running),
            json!({"TaskState": "Running"}) => Ok(OperationStatus::Running),
            json!({"TaskState": "Interrupted"}) => Ok(OperationStatus::Running),
        }
        "terminal" {
            json!({"TaskState": "Completed"}) => Ok(OperationStatus::Completed),
            json!({
                "TaskState": "Exception",
                "Messages": [
                    {"MessageId": "Base.1.0.GeneralError", "Message": "flash failed"},
                    {"MessageId": "Base.1.0.GeneralError", "Message": "rolled back"},
                ],
            }) => failed_with("Exception", "flash failed; rolled back"),
        }
        "unrecognized" {
            json!({"TaskState": "Exploded"}) => Err(()),
        }
    );
}

#[test]
fn job_states_separate_operator_waits_from_failures() {
    value_scenarios!(run = |fields: Value| job_status(&job(fields)).map_err(drop);
        "still running" {
            json!({"JobState": "Interrupted"}) => Ok(OperationStatus::Running),
        }
        "terminal" {
            json!({"JobState": "Completed"}) => Ok(OperationStatus::Completed),
            json!({
                "JobState": "Invalid",
                "Messages": [{"MessageId": "Base.1.0.GeneralError", "Message": "bad payload"}],
            }) => failed_with("Invalid", "bad payload"),
        }
        "operator" {
            json!({"JobState": "UserIntervention"}) => needs_intervention("UserIntervention"),
            json!({"JobState": "Suspended"}) => needs_intervention("Suspended"),
        }
    );
}

#[test]
fn dell_job_states_report_jobs_that_wait_for_a_host_reset() {
    value_scenarios!(run = |fields: Value| dell_job_status(&dell_job(fields)).map_err(drop);
        "awaiting reset" {
            json!({"JobState": "Scheduled", "Message": "Task successfully scheduled."})
                => Ok(OperationStatus::AwaitingReset),
            json!({"JobState": "PendingActivation"}) => Ok(OperationStatus::AwaitingReset),
        }
        "still running" {
            json!({"JobState": "Running"}) => Ok(OperationStatus::Running),
        }
        "terminal" {
            json!({"JobState": "RebootCompleted"}) => Ok(OperationStatus::Completed),
            json!({"JobState": "CompletedWithErrors", "Message": "SYS051"})
                => failed_with("CompletedWithErrors", "SYS051"),
            json!({"JobState": "Scheduled", "Message": JOB_INITIALIZATION_FAILURE})
                => failed_with("ScheduledWithErrors", JOB_INITIALIZATION_FAILURE),
        }
        "operator" {
            json!({"JobState": "Paused"}) => needs_intervention("Paused"),
        }
        "invalid" {
            json!({}) => Err(()),
            json!({"JobState": "Unknown"}) => Err(()),
        }
    );
}

#[test]
fn a_task_that_transitioned_to_a_job_names_that_job() {
    let transitioned = task(json!({
        "TaskState": "Running",
        "Messages": [{
            "MessageId": TRANSITIONED_TO_JOB,
            "MessageArgs": ["/redfish/v1/JobService/Jobs/3"],
        }],
    }));
    assert_eq!(
        transitioned_job(&transitioned),
        Some(ODataId::from("/redfish/v1/JobService/Jobs/3".to_string()))
    );
    assert_eq!(
        transitioned_job(&task(json!({"TaskState": "Running"}))),
        None
    );
}
