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
use bmc_explorer::nv_explore;
use bmc_mock::{DpuSettings, test_support};
use tokio::test;

use crate::common;

/// The identity projected from an exploration describes the system,
/// chassis and firmware versions its report does.
#[test]
async fn platform_identity_matches_the_exploration_report() {
    let cases = [
        ("Dell", test_support::dell_poweredge_r750_bmc().await, true),
        (
            "BlueField-3, ERoT chassis skipped",
            test_support::dell_poweredge_r750_bluefield3_bmc(DpuSettings::default()).await,
            true,
        ),
        ("Wiwynn GB200", test_support::wiwynn_gb200_bmc().await, true),
        (
            "Delta power shelf, no system",
            test_support::delta_powershelf_bmc().await,
            false,
        ),
    ];
    for (name, harness, has_system) in cases {
        let exploration = nv_explore(harness.service_root, &common::explorer_config())
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let (report, identity) = (exploration.report, exploration.identity);

        let system = identity.system.map(|system| system.id);
        let expected_system = has_system
            .then(|| report.systems.first().map(|system| system.id.clone()))
            .flatten();
        assert_eq!(system, expected_system, "{name}");
        assert_eq!(
            identity
                .chassis
                .into_iter()
                .map(|chassis| chassis.id)
                .collect::<Vec<_>>(),
            report
                .chassis
                .iter()
                .map(|chassis| chassis.id.clone())
                .collect::<Vec<_>>(),
            "{name}"
        );
        let report_versions = report
            .service
            .iter()
            .filter(|service| service.id == "FirmwareInventory")
            .flat_map(|service| &service.inventories)
            .map(|inventory| (inventory.id.clone(), inventory.version.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            identity
                .firmware_inventory
                .into_iter()
                .map(|inventory| (inventory.id, inventory.version))
                .collect::<Vec<_>>(),
            report_versions,
            "{name}"
        );
    }
}
