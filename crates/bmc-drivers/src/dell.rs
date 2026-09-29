/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Dell iDRAC mechanics shared by several capabilities.

use bmc_platform::{DriverOutcome, OpCx, OperationReference, PlatformError};
use nv_redfish::core::{Bmc, ModificationResponse, RedfishSettings};
use nv_redfish::oem::dell::DellManager;
use nv_redfish::oem::dell::attributes::{AttributesUpdate, DellAttributes, DellAttributesUpdate};
use nv_redfish::oem::dell::schema::ActionAnnotations;
use nv_redfish::oem::dell::schema::dell_job_service::DellJobServiceDeleteJobQueueAction;
use serde_json::{Value, json};

use crate::resources::{dynamic_properties, patch_bios_settings, selected_bios};

/// The iDRAC attribute that blocks configuration changes while enabled.
const SYSTEM_LOCKDOWN: &str = "Lockdown.1.SystemLockdown";

/// Maps a mutation response, reporting iDRAC job-queue locations as vendor jobs.
pub(crate) fn job_outcome<T>(response: ModificationResponse<T>) -> DriverOutcome {
    match response {
        ModificationResponse::Task(task)
            if task.location.0.to_string().contains("/Oem/Dell/Jobs/") =>
        {
            let retry_after_seconds = task.retry_after.map(|duration| duration.as_secs());
            match task
                .location
                .0
                .last_segment()
                .and_then(|job_id| job_id.parse().ok())
            {
                Some(job_id) => DriverOutcome::accepted(OperationReference::VendorJob {
                    uri: task.location.0,
                    job_id,
                    retry_after_seconds,
                }),
                None => DriverOutcome::accepted(OperationReference::RedfishTask {
                    uri: task.location.0,
                    retry_after_seconds,
                }),
            }
        }
        other => DriverOutcome::from(other),
    }
}

/// The Dell resources the selected Manager advertises.
fn dell_manager<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DellManager<B>, PlatformError> {
    cx.manager()?
        .oem_dell()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// The selected Manager's iDRAC attributes.
async fn manager_attributes<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DellAttributes<B>, PlatformError> {
    cx.manager()?
        .oem_dell_attributes()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

/// Deletes every queued iDRAC job.
///
/// iDRAC rejects new configuration jobs (`SYS011`) while any job is queued,
/// so every BIOS writer clears the queue first. The queue cannot be cleared
/// while system lockdown is enabled, which is reported as `LockedDown`.
pub(crate) async fn clear_job_queue<B: Bmc>(cx: &OpCx<'_, B>) -> Result<(), PlatformError> {
    let system_lockdown = manager_attributes(cx)
        .await?
        .attribute(SYSTEM_LOCKDOWN)
        .and_then(|value| value.str_value().map(str::to_owned))
        .ok_or_else(|| PlatformError::InvalidResponse {
            message: format!("iDRAC attributes do not report {SYSTEM_LOCKDOWN}"),
        })?;
    if system_lockdown == "Enabled" {
        return Err(PlatformError::LockedDown);
    }
    let service = dell_manager(cx)?
        .job_service()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .raw();
    let action = service
        .actions
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|actions| actions.delete_job_queue.as_ref())
        .ok_or(PlatformError::Unsupported)?;
    cx.action(
        action,
        &DellJobServiceDeleteJobQueueAction {
            redfish_annotations: ActionAnnotations::default(),
            job_id: "JID_CLEARALL".to_string(),
        },
    )
    .await
    .map(drop)
}

/// Creates the configuration job that applies staged BIOS settings on the next reset.
pub(crate) async fn create_bios_config_job<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<DriverOutcome, PlatformError> {
    let bios = selected_bios(cx).await?;
    let settings = bios
        .raw()
        .settings_object()
        .ok_or(PlatformError::Unsupported)?;
    dell_manager(cx)?
        .configuration_jobs()
        .ok_or(PlatformError::Unsupported)?
        .create_configuration_job(settings.id())
        .await
        .map(job_outcome)
        .map_err(|error| cx.map_redfish_error(error))
}

/// Stages BIOS attributes as an iDRAC configuration job applied on the next reset.
pub(crate) async fn stage_bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: Value,
) -> Result<DriverOutcome, PlatformError> {
    clear_job_queue(cx).await?;
    patch_bios_settings(
        cx,
        &json!({
            "@Redfish.SettingsApplyTime": {"ApplyTime": "OnReset"},
            "Attributes": attributes,
        }),
    )
    .await
    .map(job_outcome)
}

/// Writes iDRAC manager attributes, given as a JSON object.
pub(crate) async fn patch_manager_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: Value,
) -> Result<DriverOutcome, PlatformError> {
    let Value::Object(attributes) = attributes else {
        return Err(PlatformError::InvalidResponse {
            message: "iDRAC attributes must be a JSON object".to_string(),
        });
    };
    let body = DellAttributesUpdate::builder()
        .with_attributes(
            AttributesUpdate::builder()
                .with_dynamic_properties(dynamic_properties(&attributes.into_iter().collect())?)
                .build(),
        )
        .build();
    manager_attributes(cx)
        .await?
        .update(&body)
        .await
        .map(job_outcome)
        .map_err(|error| cx.map_redfish_error(error))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bmc_platform::{EtagMode, VendorJobId};
    use nv_redfish::core::{AsyncTask, AsyncTaskLocation};

    use super::*;
    use crate::test_support::{Fixture, body};

    fn task(location: &str) -> ModificationResponse<Value> {
        ModificationResponse::Task(AsyncTask {
            location: AsyncTaskLocation(location.to_string().into()),
            retry_after: Some(Duration::from_secs(7)),
        })
    }

    #[test]
    fn only_job_queue_locations_become_vendor_jobs() {
        assert_eq!(
            job_outcome(task(
                "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/Jobs/JID_42"
            )),
            DriverOutcome::accepted(OperationReference::VendorJob {
                uri: "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/Jobs/JID_42"
                    .to_string()
                    .into(),
                job_id: VendorJobId::new("JID_42".to_string()).expect("nonempty"),
                retry_after_seconds: Some(7),
            })
        );
        assert_eq!(
            job_outcome(task("/redfish/v1/TaskService/Tasks/42")),
            DriverOutcome::accepted(OperationReference::RedfishTask {
                uri: "/redfish/v1/TaskService/Tasks/42".to_string().into(),
                retry_after_seconds: Some(7),
            })
        );
    }

    #[tokio::test]
    async fn job_queue_is_cleared_through_the_advertised_action_unless_locked_down() {
        const MANAGER: &str = "/redfish/v1/Managers/iDRAC.Embedded.1";
        const ATTRIBUTES: &str =
            "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/DellAttributes/iDRAC.Embedded.1";
        const JOB_SERVICE: &str = "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/DellJobService";
        const DELETE_JOB_QUEUE: &str = "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/DellJobService/Actions/DellJobService.DeleteJobQueue";

        for (system_lockdown, expected) in [
            ("Enabled", Err(PlatformError::LockedDown)),
            ("Disabled", Ok(())),
        ] {
            let bmc = Fixture::new(
                "Dell",
                "Integrated Dell Remote Access Controller",
                "System.Embedded.1",
                "iDRAC.Embedded.1",
            )
            .document(
                MANAGER,
                json!({
                    "@odata.id": MANAGER,
                    "Id": "iDRAC.Embedded.1",
                    "Name": "Manager",
                    "Links": {"Oem": {"Dell": {
                        "DellAttributes": [{"@odata.id": ATTRIBUTES}],
                        "DellJobService": {"@odata.id": JOB_SERVICE},
                    }}},
                }),
            )
            .document(
                ATTRIBUTES,
                json!({
                    "@odata.id": ATTRIBUTES,
                    "Id": "iDRAC.Embedded.1",
                    "Name": "Manager Attributes",
                    "Attributes": {"Lockdown.1.SystemLockdown": system_lockdown},
                }),
            )
            .document(
                JOB_SERVICE,
                json!({
                    "@odata.id": JOB_SERVICE,
                    "Id": "Job Service",
                    "Name": "DellJobService",
                    "Actions": {"#DellJobService.DeleteJobQueue": {"target": DELETE_JOB_QUEUE}},
                }),
            )
            .build()
            .await;
            let cx = bmc.cx(EtagMode::Resource).await;

            assert_eq!(clear_job_queue(&cx).await, expected, "{system_lockdown}");
            let writes = bmc.writes();
            match expected {
                Err(_) => assert!(writes.is_empty(), "{system_lockdown}"),
                Ok(()) => {
                    assert_eq!(writes.len(), 1);
                    assert!(
                        writes[0].uri.ends_with(DELETE_JOB_QUEUE),
                        "{}",
                        writes[0].uri
                    );
                    assert_eq!(body(&writes[0]), json!({"JobID": "JID_CLEARALL"}));
                }
            }
        }
    }
}
