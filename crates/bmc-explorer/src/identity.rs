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

//! The [`PlatformIdentity`] driver selection matches on, projected from the
//! resources exploration already read.

use bmc_platform::{ManagerIdentity, PlatformIdentity, ServiceRootIdentity, SystemIdentity};
use nv_redfish::computer_system::ComputerSystem;
use nv_redfish::manager::Manager;
use nv_redfish::{Bmc, ServiceRoot};

use crate::chassis::ExploredChassisCollection;
use crate::hw::HwType;
use crate::inventories::ExploredInventories;

/// The ServiceRoot part of the identity, the only part a BMC still on its
/// factory password serves.
pub fn service_root_identity<B: Bmc>(root: &ServiceRoot<B>) -> ServiceRootIdentity {
    ServiceRootIdentity {
        vendor: root.vendor().map(|vendor| vendor.into_inner().to_string()),
        product: root
            .product()
            .map(|product| product.into_inner().to_string()),
        oem_keys: root
            .root
            .oem
            .as_ref()
            .and_then(|oem| oem.additional_properties.as_object())
            .map(|oem| oem.keys().cloned().collect())
            .unwrap_or_default(),
    }
}

/// The identity of the explored resources; `system` is `None` on platforms
/// that list no ComputerSystem.
pub(crate) fn project<B: Bmc>(
    root: &ServiceRoot<B>,
    system: Option<&ComputerSystem<B>>,
    manager: &Manager<B>,
    chassis: &ExploredChassisCollection<B>,
    inventories: &ExploredInventories<B>,
    hw_type: Option<HwType>,
) -> PlatformIdentity {
    let manager = manager.raw();
    PlatformIdentity {
        service_root: service_root_identity(root),
        system: system.map(|system| {
            let system = system.raw();
            SystemIdentity {
                id: system.id.clone(),
                manufacturer: system.manufacturer.clone().flatten(),
                model: system.model.clone().flatten(),
                sku: system.sku.clone().flatten(),
                part_number: system.part_number.clone().flatten(),
                bios_version: system.bios_version.clone().flatten(),
            }
        }),
        manager: Some(ManagerIdentity {
            id: manager.id.clone(),
            model: manager.model.clone().flatten(),
            firmware: manager.firmware_version.clone().flatten(),
        }),
        chassis: chassis.identities(),
        firmware_inventory: inventories.identities(hw_type),
    }
}
