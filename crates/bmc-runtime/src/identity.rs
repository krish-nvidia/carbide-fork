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

use std::sync::Arc;

use bmc_platform::{
    ChassisIdentity, ManagerIdentity, PlatformIdentity, ServiceRootIdentity, SystemIdentity,
};
use nv_redfish::{Bmc, Resource, ServiceRoot};
use serde_json::Value;

use crate::IdentityProjectionError;

/// Projects driver-selection identity from a live generic Redfish service root.
///
/// The first manager is used. A system with a non-empty BIOS version is
/// preferred over auxiliary systems; otherwise the first system is used.
pub async fn project_platform_identity<B: Bmc>(
    root: Arc<ServiceRoot<B>>,
) -> Result<PlatformIdentity, IdentityProjectionError> {
    let manager = project_manager(root.as_ref()).await?;
    let system = project_system(root.as_ref()).await?;
    let chassis = project_chassis(root.as_ref()).await?;
    let oem_keys = collect_oem_keys(
        root.resource_ref()
            .base
            .oem
            .as_ref()
            .map(|oem| &oem.additional_properties),
    );

    Ok(PlatformIdentity {
        service_root: ServiceRootIdentity {
            vendor: root.vendor().map(|value| value.to_string()),
            product: root.product().map(|value| value.to_string()),
            oem_keys,
        },
        manager,
        system,
        chassis,
    })
}

fn collect_oem_keys(oem: Option<&Value>) -> Vec<String> {
    let mut keys: Vec<String> = oem
        .and_then(Value::as_object)
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

async fn project_manager<B: Bmc>(
    root: &ServiceRoot<B>,
) -> Result<Option<ManagerIdentity>, IdentityProjectionError> {
    let Some(collection) = root
        .managers()
        .await
        .map_err(|error| IdentityProjectionError::new("manager collection", error))?
    else {
        return Ok(None);
    };
    let manager = collection
        .members()
        .await
        .map_err(|error| IdentityProjectionError::new("manager members", error))?
        .into_iter()
        .next();

    Ok(manager.map(|manager| {
        let raw = manager.raw();
        ManagerIdentity {
            model: raw.model.clone().flatten(),
            firmware: raw.firmware_version.clone().flatten(),
        }
    }))
}

async fn project_system<B: Bmc>(
    root: &ServiceRoot<B>,
) -> Result<Option<SystemIdentity>, IdentityProjectionError> {
    let Some(collection) = root
        .systems()
        .await
        .map_err(|error| IdentityProjectionError::new("system collection", error))?
    else {
        return Ok(None);
    };
    let systems = collection
        .members()
        .await
        .map_err(|error| IdentityProjectionError::new("system members", error))?;
    let selected = systems
        .iter()
        .position(|system| {
            system
                .raw()
                .bios_version
                .as_ref()
                .and_then(Option::as_ref)
                .is_some_and(|version| !version.trim().is_empty())
        })
        .or((!systems.is_empty()).then_some(0))
        .map(|index| &systems[index]);

    Ok(selected.map(|system| {
        let hardware = system.hardware_id();
        let raw = system.raw();
        SystemIdentity {
            id: system.id().to_string(),
            manufacturer: hardware.manufacturer.map(|value| value.to_string()),
            model: hardware.model.map(|value| value.to_string()),
            sku: system.sku().map(|value| value.to_string()),
            part_number: hardware.part_number.map(|value| value.to_string()),
            bios_version: raw.bios_version.clone().flatten(),
        }
    }))
}

async fn project_chassis<B: Bmc>(
    root: &ServiceRoot<B>,
) -> Result<Vec<ChassisIdentity>, IdentityProjectionError> {
    let Some(collection) = root
        .chassis()
        .await
        .map_err(|error| IdentityProjectionError::new("chassis collection", error))?
    else {
        return Ok(Vec::new());
    };
    collection
        .members()
        .await
        .map_err(|error| IdentityProjectionError::new("chassis members", error))
        .map(|members| {
            members
                .into_iter()
                .map(|chassis| {
                    let hardware = chassis.hardware_id();
                    ChassisIdentity {
                        id: chassis.id().to_string(),
                        manufacturer: hardware.manufacturer.map(|value| value.to_string()),
                        model: hardware.model.map(|value| value.to_string()),
                        part_number: hardware.part_number.map(|value| value.to_string()),
                    }
                })
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::collect_oem_keys;

    #[test]
    fn projection_collects_all_oem_keys_in_stable_order() {
        let oem = json!({
            "Nvidia": {},
            "Dell": {},
            "Acme": {}
        });

        assert_eq!(
            collect_oem_keys(Some(&oem)),
            vec!["Acme".to_string(), "Dell".to_string(), "Nvidia".to_string()]
        );
        assert!(collect_oem_keys(None).is_empty());
    }
}
