/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Dell iDRAC mechanics shared by several capabilities.

use bmc_platform::{DriverOutcome, OpCx, OperationReference, PlatformError};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ModificationResponse, ODataId, RedfishSettings};
use serde_json::{Value, json};

use crate::resources::{patch_bios_settings, selected_bios};

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

/// Deletes every queued iDRAC job.
///
/// iDRAC rejects new configuration jobs (`SYS011`) while any job is queued,
/// so every BIOS writer clears the queue first.
pub(crate) async fn clear_job_queue<B: Bmc>(cx: &OpCx<'_, B>) -> Result<(), PlatformError> {
    let manager = cx.manager()?;
    let target = ODataId::from(format!(
        "{}/Oem/Dell/DellJobService/Actions/DellJobService.DeleteJobQueue",
        manager.odata_id()
    ));
    cx.post(&target, &json!({"JobID": "JID_CLEARALL"}))
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
    let manager = cx.manager()?;
    let jobs = ODataId::from(format!("{}/Oem/Dell/Jobs", manager.odata_id()));
    cx.post_response(
        &jobs,
        &json!({"TargetSettingsURI": settings.id().to_string()}),
    )
    .await
    .map(job_outcome)
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

/// Writes iDRAC manager attributes.
///
/// `nv-redfish` exposes `DellAttributes` read-only and reaches it through
/// this same derived path, since iDRAC does not link it from the Manager.
/// iDRAC accepts these writes without `If-Match`.
pub(crate) async fn patch_manager_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attributes: Value,
) -> Result<DriverOutcome, PlatformError> {
    let manager = cx.manager()?;
    let id = ODataId::from(format!(
        "{}/Oem/Dell/DellAttributes/{}",
        manager.odata_id(),
        manager.id()
    ));
    cx.patch_id(&id, None, &json!({"Attributes": attributes}))
        .await
        .map(job_outcome)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bmc_platform::VendorJobId;
    use nv_redfish::core::{AsyncTask, AsyncTaskLocation};

    use super::*;

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
}
