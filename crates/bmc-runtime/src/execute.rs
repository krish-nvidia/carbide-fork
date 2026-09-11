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

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use bmc_platform::{
    Capability, ClassifyBmcError, ControllerAction, DriverOutcome, Fetched, OpCx,
    OperationReference, PlatformError,
};
use nv_redfish::Bmc;
use nv_redfish::core::ODataId;
use nv_redfish::schema::task::{Task, TaskState};
use serde::Deserialize;
use thiserror::Error;

use crate::{ConnectedBmc, DriverTable, DriverTableError};

/// Where an operation stands after the executor has done what it can.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Progress {
    /// The operation and every follow-up action completed.
    Complete,
    /// Prerequisites were performed; the caller must retry the operation.
    Retry,
    /// `action` needs the controller; `remaining` are the actions queued after
    /// it, so the controller can resume them once it has acted.
    Deferred {
        action: ControllerAction,
        remaining: Vec<ControllerAction>,
    },
}

/// Failure while driving an outcome to completion.
#[derive(Debug, Error)]
pub enum ExecuteError {
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error(transparent)]
    Drivers(#[from] DriverTableError),
    /// Asynchronous BMC work ended in a failed state.
    #[error("BMC work {uri} ended in state {state}")]
    Failed { uri: ODataId, state: String },
    /// Asynchronous BMC work did not finish within the executor's budget.
    #[error("BMC work {uri} did not complete in time")]
    Timeout { uri: ODataId },
    /// Drivers kept requesting actions past the executor's budget.
    #[error("executor budget exhausted before running {action:?}")]
    BudgetExhausted { action: ControllerAction },
}

/// Drives [`DriverOutcome`]s to completion: polls tasks and vendor jobs and
/// performs the follow-up and prerequisite actions drivers ask for.
///
/// Actions that need the controller ([`ControllerAction::RefreshExploration`],
/// [`ControllerAction::ManualIntervention`]) stop execution and are returned.
/// The budget bounds both wall-clock time and the number of driver actions,
/// so a prerequisite cycle between drivers fails instead of recursing forever.
pub struct Executor<'a, B: Bmc + 'static> {
    bmc: &'a ConnectedBmc<B>,
    drivers: &'a DriverTable<B>,
    poll_interval: Duration,
    deadline: Instant,
    remaining_actions: AtomicU32,
}

/// A vendor job-service entry. Today only Dell iDRAC produces
/// [`OperationReference::VendorJob`], and its entries report `JobState`
/// rather than a Redfish `TaskState`; a second vendor must extend this
/// vocabulary. A body without `JobState` is an invalid response, not a
/// running job.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VendorJob {
    job_state: JobState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum JobState {
    Completed,
    Failed,
    CompletedWithErrors,
    RebootFailed,
    #[serde(other)]
    Running,
}

enum WorkState {
    Running,
    Done,
    Failed(String),
}

fn task_state(state: Option<TaskState>) -> WorkState {
    match state {
        Some(TaskState::Completed) => WorkState::Done,
        Some(
            failed @ (TaskState::Exception
            | TaskState::Killed
            | TaskState::Cancelled
            | TaskState::Interrupted),
        ) => WorkState::Failed(format!("{failed:?}")),
        _ => WorkState::Running,
    }
}

fn job_state(state: JobState) -> WorkState {
    match state {
        JobState::Completed => WorkState::Done,
        JobState::Running => WorkState::Running,
        failed => WorkState::Failed(format!("{failed:?}")),
    }
}

impl<'a, B: Bmc + 'static> Executor<'a, B>
where
    B::Error: ClassifyBmcError,
{
    /// Actions one drive may perform unless [`Executor::with_max_actions`] says otherwise.
    pub const DEFAULT_MAX_ACTIONS: u32 = 16;

    /// Floor for `Retry-After`, so a BMC advertising zero cannot make the
    /// executor poll in a tight loop.
    pub const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);

    /// Creates an executor that gives up after `budget` of wall-clock time.
    pub fn new(bmc: &'a ConnectedBmc<B>, drivers: &'a DriverTable<B>, budget: Duration) -> Self {
        Self {
            bmc,
            drivers,
            poll_interval: Duration::from_secs(5),
            deadline: Instant::now() + budget,
            remaining_actions: AtomicU32::new(Self::DEFAULT_MAX_ACTIONS),
        }
    }

    /// Overrides the poll interval used when the BMC suggests none.
    pub const fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Overrides how many driver actions one drive may perform.
    pub fn with_max_actions(self, max_actions: u32) -> Self {
        self.remaining_actions.store(max_actions, Ordering::Relaxed);
        self
    }

    /// Drives one outcome, then its follow-up or prerequisite actions in order.
    ///
    /// The selected system and manager are resolved once for the whole drive.
    pub async fn drive(&self, outcome: DriverOutcome) -> Result<Progress, ExecuteError> {
        let cx = self.bmc.operation_context().await?;
        self.drive_with(&cx, outcome).await
    }

    async fn drive_with(
        &self,
        cx: &OpCx<'a, B>,
        outcome: DriverOutcome,
    ) -> Result<Progress, ExecuteError> {
        match outcome {
            DriverOutcome::Complete { follow_up } => self.run_all(cx, follow_up).await,
            DriverOutcome::Accepted {
                reference,
                additional_references,
                follow_up,
            } => {
                let mut first_error = self.wait(&reference).await.err();
                for reference in additional_references {
                    if let Err(error) = self.wait(&reference).await {
                        first_error.get_or_insert(error);
                    }
                }
                if let Some(error) = first_error {
                    return Err(error);
                }
                self.run_all(cx, follow_up).await
            }
            DriverOutcome::Blocked {
                prerequisite,
                additional_prerequisites,
            } => {
                let mut prerequisites = vec![prerequisite];
                prerequisites.extend(additional_prerequisites);
                match self.run_all(cx, prerequisites).await? {
                    Progress::Complete => Ok(Progress::Retry),
                    other => Ok(other),
                }
            }
        }
    }

    /// Polls `reference` until it completes. Polling only needs the raw
    /// transport, so no system or manager is resolved here.
    async fn wait(&self, reference: &OperationReference) -> Result<(), ExecuteError> {
        let interval = reference
            .retry_after_seconds()
            .map_or(self.poll_interval, Duration::from_secs)
            .max(Self::MIN_POLL_INTERVAL);
        loop {
            let state = match reference {
                OperationReference::RedfishTask { uri, .. } => {
                    task_state(self.get::<Task>(uri).await?.task_state)
                }
                OperationReference::VendorJob { uri, .. } => {
                    job_state(self.get::<VendorJob>(uri).await?.job_state)
                }
            };
            match state {
                WorkState::Done => return Ok(()),
                WorkState::Failed(state) => {
                    return Err(ExecuteError::Failed {
                        uri: reference.uri().clone(),
                        state,
                    });
                }
                WorkState::Running => {}
            }
            if Instant::now() + interval > self.deadline {
                return Err(ExecuteError::Timeout {
                    uri: reference.uri().clone(),
                });
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn get<T>(&self, uri: &ODataId) -> Result<Arc<Fetched<T>>, ExecuteError>
    where
        T: for<'de> Deserialize<'de> + Send + Sync + 'static,
    {
        self.bmc
            .bmc()
            .get::<Fetched<T>>(uri)
            .await
            .map_err(|error| error.classify().into())
    }

    async fn run_all(
        &self,
        cx: &OpCx<'a, B>,
        actions: Vec<ControllerAction>,
    ) -> Result<Progress, ExecuteError> {
        let mut queue = actions.into_iter();
        while let Some(action) = queue.next() {
            match self.run(cx, action).await? {
                Progress::Complete => {}
                Progress::Retry => return Ok(Progress::Retry),
                Progress::Deferred {
                    action,
                    mut remaining,
                } => {
                    remaining.extend(queue);
                    return Ok(Progress::Deferred { action, remaining });
                }
            }
        }
        Ok(Progress::Complete)
    }

    fn charge(&self, action: &ControllerAction) -> Result<(), ExecuteError> {
        let remaining = self.remaining_actions.load(Ordering::Relaxed);
        if remaining == 0 || Instant::now() > self.deadline {
            return Err(ExecuteError::BudgetExhausted {
                action: action.clone(),
            });
        }
        self.remaining_actions
            .store(remaining - 1, Ordering::Relaxed);
        Ok(())
    }

    // Actions produce outcomes of their own, so this recurses through `drive`;
    // `charge` bounds the recursion.
    fn run<'c>(
        &'c self,
        cx: &'c OpCx<'a, B>,
        action: ControllerAction,
    ) -> Pin<Box<dyn Future<Output = Result<Progress, ExecuteError>> + Send + 'c>> {
        Box::pin(async move {
            if matches!(
                action,
                ControllerAction::RefreshExploration | ControllerAction::ManualIntervention { .. }
            ) {
                return Ok(Progress::Deferred {
                    action,
                    remaining: Vec::new(),
                });
            }
            self.charge(&action)?;
            let map = self.bmc.endpoint().driver_map();
            let outcome = match action {
                ControllerAction::Wait { seconds } => {
                    tokio::time::sleep(Duration::from_secs(seconds.get())).await;
                    return Ok(Progress::Complete);
                }
                ControllerAction::Power(reset_type) => {
                    self.drivers
                        .power(map.get(Capability::Power))?
                        .set(cx, reset_type)
                        .await?
                }
                ControllerAction::BmcReset => {
                    self.drivers
                        .bmc_control(map.get(Capability::BmcControl))?
                        .reset(cx)
                        .await?
                }
                ControllerAction::SetLockdown { scope, state } => {
                    self.drivers
                        .lockdown(map.get(Capability::Lockdown))?
                        .set(cx, scope, state)
                        .await?
                }
                ControllerAction::ClearNvram => {
                    self.drivers
                        .bios(map.get(Capability::Bios))?
                        .reset(cx)
                        .await?
                }
                ControllerAction::RefreshExploration
                | ControllerAction::ManualIntervention { .. } => unreachable!("deferred above"),
            };
            self.drive_with(cx, outcome).await
        })
    }
}

#[cfg(test)]
mod tests;
