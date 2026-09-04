/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Storage capability drivers.

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, Storage};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId, Reference};
use serde::Serialize;

use crate::dell::job_outcome;

/// Dell iDRAC boot-storage driver.
pub(crate) struct IdracBossStorage;

#[derive(Serialize)]
struct DecommissionRequest {
    #[serde(rename = "@Redfish.OperationApplyTime")]
    operation_apply_time: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct VolumeLinks {
    drives: Vec<Reference>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct CreateVolumeRequest<'a> {
    name: &'a str,
    #[serde(rename = "RAIDType")]
    raid_type: &'static str,
    links: VolumeLinks,
}

fn raid_type(drive_count: usize) -> Result<&'static str, PlatformError> {
    match drive_count {
        1 => Ok("RAID0"),
        2 => Ok("RAID1"),
        count => Err(PlatformError::InvalidResponse {
            message: format!("Dell BOSS requires one or two drives, found {count}"),
        }),
    }
}

async fn find_controller<B: Bmc>(
    cx: &OpCx<'_, B>,
    controller_id: &str,
) -> Result<nv_redfish::computer_system::Storage<B>, PlatformError> {
    let controllers = cx
        .system()?
        .storage_controllers()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .unwrap_or_default();
    if let Some(controller) = controllers
        .into_iter()
        .find(|controller| controller.odata_id().last_segment() == Some(controller_id))
    {
        return Ok(controller);
    }

    Err(PlatformError::InvalidResponse {
        message: format!("storage controller {controller_id} was not found"),
    })
}

#[async_trait]
impl<B: Bmc> Storage<B> for IdracBossStorage {
    async fn boot_controller(&self, cx: &OpCx<'_, B>) -> Result<Option<String>, PlatformError> {
        let controllers = cx
            .system()?
            .storage_controllers()
            .await
            .map_err(|error| cx.map_redfish_error(error))?
            .unwrap_or_default();
        for controller in controllers {
            let id = controller.odata_id();
            if id.to_string().contains("BOSS") {
                return id
                    .last_segment()
                    .map(str::to_owned)
                    .map(Some)
                    .ok_or_else(|| PlatformError::InvalidResponse {
                        message: format!("BOSS controller URI has no identifier: {id}"),
                    });
            }
        }

        Ok(None)
    }

    async fn decommission_controller(
        &self,
        cx: &OpCx<'_, B>,
        controller_id: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        let controller = find_controller(cx, controller_id).await?;
        let target = ODataId::from(format!(
            "{}/Actions/Oem/DellStorage.ControllerDrivesDecommission",
            controller.odata_id()
        ));
        let response = cx
            .post_response(
                &target,
                &DecommissionRequest {
                    operation_apply_time: "Immediate",
                },
            )
            .await?;
        Ok(job_outcome(response))
    }

    async fn create_volume(
        &self,
        cx: &OpCx<'_, B>,
        controller_id: &str,
        volume_name: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        if volume_name.is_empty() || volume_name.len() > 15 {
            return Err(PlatformError::InvalidResponse {
                message: "Dell volume name must contain 1 through 15 bytes".to_string(),
            });
        }

        let controller = find_controller(cx, controller_id).await?;
        let raw = controller.raw();
        let drive_refs = raw
            .drives
            .as_ref()
            .ok_or_else(|| PlatformError::InvalidResponse {
                message: format!("controller {controller_id} does not report its drives"),
            })?;
        let request = CreateVolumeRequest {
            name: volume_name,
            raid_type: raid_type(drive_refs.len())?,
            links: VolumeLinks {
                drives: drive_refs.iter().map(Reference::from).collect(),
            },
        };
        let target = ODataId::from(format!("{}/Volumes", controller.odata_id()));
        let response = cx.post_response(&target, &request).await?;
        Ok(job_outcome(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_drive_count_to_boss_raid_level() {
        assert_eq!(raid_type(1), Ok("RAID0"));
        assert_eq!(raid_type(2), Ok("RAID1"));
        assert!(raid_type(0).is_err());
        assert!(raid_type(3).is_err());
    }
}
