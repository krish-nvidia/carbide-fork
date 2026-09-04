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
use std::num::NonZeroU64;
use std::str::FromStr;

use nv_redfish::core::{ModificationResponse, ODataId};
use nv_redfish::resource::ResetType;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capabilities::{LockdownDesiredState, LockdownScope};

/// Rejects empty or whitespace-only identifiers shared by the newtypes below.
fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// Vendor job identifier, such as an iDRAC `JID_…`; never empty.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct VendorJobId(String);

impl VendorJobId {
    /// Validates a job identifier; whitespace-only values are rejected.
    pub fn new(value: String) -> Result<Self, VendorJobIdError> {
        non_empty(value).map(Self).ok_or(VendorJobIdError)
    }

    /// Returns the identifier as the BMC reported it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VendorJobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for VendorJobId {
    type Err = VendorJobIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value.to_owned())
    }
}

impl TryFrom<String> for VendorJobId {
    type Error = VendorJobIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("vendor job id must not be empty")]
pub struct VendorJobIdError;

/// Operator-facing code naming the manual step a driver requires; never empty.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ManualInterventionCode(String);

impl ManualInterventionCode {
    /// Validates a code; whitespace-only values are rejected.
    pub fn new(value: String) -> Result<Self, ManualInterventionCodeError> {
        non_empty(value)
            .map(Self)
            .ok_or(ManualInterventionCodeError)
    }

    /// Returns the code as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ManualInterventionCode {
    type Err = ManualInterventionCodeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value.to_owned())
    }
}

impl TryFrom<String> for ManualInterventionCode {
    type Error = ManualInterventionCodeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("manual-intervention code must not be empty")]
pub struct ManualInterventionCodeError;

/// Persistable reference to asynchronous BMC work.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationReference {
    /// A standard Redfish task, polled at `uri`.
    RedfishTask {
        uri: ODataId,
        retry_after_seconds: Option<u64>,
    },
    /// A vendor job service entry, polled at `uri` and identified by `job_id`.
    VendorJob {
        uri: ODataId,
        job_id: VendorJobId,
        retry_after_seconds: Option<u64>,
    },
}

impl OperationReference {
    /// Resource to poll for completion.
    pub const fn uri(&self) -> &ODataId {
        match self {
            Self::RedfishTask { uri, .. } | Self::VendorJob { uri, .. } => uri,
        }
    }

    /// Poll interval the BMC suggested, if any.
    pub const fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Self::RedfishTask {
                retry_after_seconds,
                ..
            }
            | Self::VendorJob {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
        }
    }
}

/// Immediate normalized result of a mutation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "details", rename_all = "snake_case")]
pub enum DriverOutcome {
    /// The requested state is satisfied, including already-satisfied no-op mutations.
    Complete { follow_up: Vec<ControllerAction> },
    /// The BMC accepted asynchronous work; `follow_up` runs after it completes.
    Accepted {
        reference: OperationReference,
        follow_up: Vec<ControllerAction>,
    },
    /// The prerequisites must be performed before the operation is retried.
    Blocked {
        prerequisite: ControllerAction,
        additional_prerequisites: Vec<ControllerAction>,
    },
}

impl DriverOutcome {
    /// The requested state holds and nothing else needs to run.
    pub fn complete() -> Self {
        Self::Complete {
            follow_up: Vec::new(),
        }
    }

    /// The BMC accepted asynchronous work to poll at `reference`, with no follow-up.
    pub fn accepted(reference: OperationReference) -> Self {
        Self::Accepted {
            reference,
            follow_up: Vec::new(),
        }
    }

    /// Appends actions to run once this outcome's work has completed.
    ///
    /// A blocked outcome keeps its prerequisites; the follow-up belongs to the
    /// retried operation, not to the prerequisite.
    pub fn then(self, actions: impl IntoIterator<Item = ControllerAction>) -> Self {
        match self {
            Self::Complete { mut follow_up } => {
                follow_up.extend(actions);
                Self::Complete { follow_up }
            }
            Self::Accepted {
                reference,
                mut follow_up,
            } => {
                follow_up.extend(actions);
                Self::Accepted {
                    reference,
                    follow_up,
                }
            }
            blocked @ Self::Blocked { .. } => blocked,
        }
    }

    /// One prerequisite must run before the operation is retried.
    pub fn blocked(prerequisite: ControllerAction) -> Self {
        Self::Blocked {
            prerequisite,
            additional_prerequisites: Vec::new(),
        }
    }

    /// Combines the outcomes of two writes that were both issued.
    ///
    /// A blocked outcome wins because its prerequisite must run before either
    /// write is retried. Otherwise accepted work outranks completion so the
    /// caller keeps polling it, and follow-ups are concatenated in order. When
    /// both writes were accepted the first reference is kept; the second job
    /// keeps running on the BMC but is not polled.
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (blocked @ Self::Blocked { .. }, _) | (_, blocked @ Self::Blocked { .. }) => blocked,
            (Self::Complete { follow_up }, other) => other.prepend(follow_up),
            (accepted @ Self::Accepted { .. }, Self::Complete { follow_up })
            | (accepted @ Self::Accepted { .. }, Self::Accepted { follow_up, .. }) => {
                accepted.then(follow_up)
            }
        }
    }

    fn prepend(self, mut actions: Vec<ControllerAction>) -> Self {
        match self {
            Self::Complete { follow_up } => {
                actions.extend(follow_up);
                Self::Complete { follow_up: actions }
            }
            Self::Accepted {
                reference,
                follow_up,
            } => {
                actions.extend(follow_up);
                Self::Accepted {
                    reference,
                    follow_up: actions,
                }
            }
            blocked @ Self::Blocked { .. } => blocked,
        }
    }
}

/// A Redfish mutation response is complete unless the BMC returned a task.
impl<T> From<ModificationResponse<T>> for DriverOutcome {
    fn from(response: ModificationResponse<T>) -> Self {
        match response {
            ModificationResponse::Entity(_) | ModificationResponse::Empty => Self::complete(),
            ModificationResponse::Task(task) => Self::accepted(OperationReference::RedfishTask {
                uri: task.location.0,
                retry_after_seconds: task.retry_after.map(|duration| duration.as_secs()),
            }),
        }
    }
}

/// Persistable orchestration requested by a capability driver.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "details", rename_all = "snake_case")]
pub enum ControllerAction {
    Power(ResetType),
    BmcReset,
    SetLockdown {
        scope: LockdownScope,
        state: LockdownDesiredState,
    },
    ClearNvram,
    RefreshExploration,
    Wait {
        seconds: NonZeroU64,
    },
    ManualIntervention {
        code: ManualInterventionCode,
    },
}

#[cfg(test)]
mod tests;
