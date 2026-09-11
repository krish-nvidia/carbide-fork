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

use std::sync::Mutex;

use async_trait::async_trait;
use bmc_mock::test_support::{TestBmc, dell_poweredge_r750_bmc};
use bmc_platform::{
    CapabilitySelection, ControllerAction, DriverMap, DriverOutcome, EtagMode,
    ManualInterventionCode, OpCx, OperationReference, PlatformError, PlatformIdentity, Power,
    SystemIdentity,
};
use carbide_secrets::credentials::{BmcCredentialType, CredentialKey};
use mac_address::MacAddress;
use nv_redfish::core::ODataId;
use nv_redfish::resource::{PowerState, ResetType};

use super::*;
use crate::{AnyDriver, BmcRef, ResolvedSelection, RuleSet};

/// A power driver that replays scripted outcomes and records what it was asked.
struct ScriptedPower {
    outcomes: Mutex<Vec<DriverOutcome>>,
    calls: Mutex<Vec<ResetType>>,
}

#[async_trait]
impl Power<TestBmc> for ScriptedPower {
    async fn state(&self, _cx: &OpCx<'_, TestBmc>) -> Result<PowerState, PlatformError> {
        Ok(PowerState::On)
    }

    async fn ac_power_cycle_supported(
        &self,
        _cx: &OpCx<'_, TestBmc>,
    ) -> Result<bool, PlatformError> {
        Ok(false)
    }

    async fn set(
        &self,
        _cx: &OpCx<'_, TestBmc>,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        self.calls.lock().expect("calls lock").push(reset_type);
        let mut outcomes = self.outcomes.lock().expect("outcomes lock");
        Ok(if outcomes.is_empty() {
            DriverOutcome::complete()
        } else {
            outcomes.remove(0)
        })
    }

    async fn chassis_reset(
        &self,
        _cx: &OpCx<'_, TestBmc>,
        _chassis_id: &str,
        _reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }
}

async fn harness(
    outcomes: Vec<DriverOutcome>,
) -> (
    ConnectedBmc<TestBmc>,
    DriverTable<TestBmc>,
    &'static ScriptedPower,
) {
    let bmc = dell_poweredge_r750_bmc().await;
    let power: &'static ScriptedPower = Box::leak(Box::new(ScriptedPower {
        outcomes: Mutex::new(outcomes),
        calls: Mutex::new(Vec::new()),
    }));
    let table = DriverTable::new([(None, AnyDriver::Power(power))]).expect("table builds");
    let endpoint = BmcRef::new(
        "192.0.2.10:443".parse().expect("socket address"),
        CredentialKey::BmcCredentials {
            credential_type: BmcCredentialType::BmcRoot {
                bmc_mac_address: MacAddress::new([2, 0, 0, 0, 0, 1]),
            },
        },
        PlatformIdentity {
            system: Some(SystemIdentity {
                id: "System.Embedded.1".to_string(),
                ..SystemIdentity::default()
            }),
            ..PlatformIdentity::default()
        },
        EtagMode::default(),
        ResolvedSelection {
            drivers: DriverMap::filled(CapabilitySelection::Unsupported)
                .with(Capability::Power, CapabilitySelection::Standard),
            matched_rules: Vec::new(),
            rule_set_hash: RuleSet::new(Vec::new())
                .expect("empty rules are valid")
                .hash(),
        },
    )
    .expect("endpoint");
    let connected = ConnectedBmc::new(endpoint, bmc.bmc.clone(), bmc.service_root, None);
    (connected, table, power)
}

fn power(reset_type: ResetType) -> ControllerAction {
    ControllerAction::Power(reset_type)
}

#[tokio::test]
async fn follow_ups_run_in_order_and_blocked_becomes_retry() {
    let (connected, table, driver) = harness(vec![]).await;
    let executor = Executor::new(&connected, &table, Duration::from_secs(5));

    let progress = executor
        .drive(DriverOutcome::complete().then([power(ResetType::ForceOff), power(ResetType::On)]))
        .await
        .expect("follow-ups complete");
    assert_eq!(progress, Progress::Complete);
    assert_eq!(
        *driver.calls.lock().expect("calls"),
        vec![ResetType::ForceOff, ResetType::On]
    );

    let progress = executor
        .drive(DriverOutcome::blocked(power(ResetType::ForceOff)))
        .await
        .expect("prerequisite completes");
    assert_eq!(progress, Progress::Retry);
}

#[tokio::test]
async fn deferred_actions_carry_the_rest_of_the_queue() {
    let (connected, table, _) = harness(vec![]).await;
    let executor = Executor::new(&connected, &table, Duration::from_secs(5));
    let code = ManualInterventionCode::new("replace-psu".to_string()).expect("code");
    let progress = executor
        .drive(DriverOutcome::complete().then([
            ControllerAction::ManualIntervention { code: code.clone() },
            power(ResetType::On),
        ]))
        .await
        .expect("deferral is not an error");
    assert_eq!(
        progress,
        Progress::Deferred {
            action: ControllerAction::ManualIntervention { code },
            remaining: vec![power(ResetType::On)],
        }
    );
}

#[tokio::test]
async fn a_prerequisite_cycle_exhausts_the_action_budget() {
    let looping = std::iter::repeat_with(|| DriverOutcome::blocked(power(ResetType::ForceOff)))
        .take(100)
        .collect();
    let (connected, table, driver) = harness(looping).await;
    let executor = Executor::new(&connected, &table, Duration::from_secs(5)).with_max_actions(3);
    let error = executor
        .drive(DriverOutcome::blocked(power(ResetType::ForceOff)))
        .await
        .expect_err("cycle is cut off");
    assert!(
        matches!(error, ExecuteError::BudgetExhausted { .. }),
        "{error}"
    );
    assert_eq!(driver.calls.lock().expect("calls").len(), 3);
}

#[tokio::test]
async fn every_accepted_reference_is_polled_before_follow_ups() {
    let (connected, table, driver) = harness(vec![]).await;
    let executor = Executor::new(&connected, &table, Duration::from_secs(5));
    let outcome = DriverOutcome::Accepted {
        reference: OperationReference::RedfishTask {
            uri: ODataId::from("/redfish/v1/TaskService/Tasks/42".to_string()),
            retry_after_seconds: None,
        },
        additional_references: vec![OperationReference::RedfishTask {
            uri: ODataId::from("/redfish/v1/not-found".to_string()),
            retry_after_seconds: None,
        }],
        follow_up: vec![power(ResetType::On)],
    };

    assert!(executor.drive(outcome).await.is_err());
    assert!(driver.calls.lock().expect("calls").is_empty());
}

#[test]
fn task_and_job_states_classify_completion_and_failure() {
    assert!(matches!(
        task_state(Some(TaskState::Completed)),
        WorkState::Done
    ));
    assert!(matches!(
        task_state(Some(TaskState::Running)),
        WorkState::Running
    ));
    assert!(matches!(task_state(None), WorkState::Running));
    assert!(matches!(
        task_state(Some(TaskState::Exception)),
        WorkState::Failed(state) if state == "Exception"
    ));
    let job = |body: &str| {
        serde_json::from_str::<VendorJob>(body)
            .expect("job body")
            .job_state
    };
    assert!(matches!(
        job_state(job(r#"{"JobState":"Completed"}"#)),
        WorkState::Done
    ));
    assert!(matches!(
        job_state(job(r#"{"JobState":"Scheduled"}"#)),
        WorkState::Running
    ));
    assert!(matches!(
        job_state(job(r#"{"JobState":"RebootFailed"}"#)),
        WorkState::Failed(_)
    ));
}
