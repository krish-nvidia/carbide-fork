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
//! In-process coordination for endpoint exploration.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use carbide_redfish::boot_interface::BootInterfaceTarget;
use libredfish::RoleId;
use mac_address::MacAddress;
use model::expected_entity::ExpectedEntity;
use model::machine::MachineInterfaceSnapshot;
use model::site_explorer::{
    EndpointExplorationError, EndpointExplorationReport, LockdownStatus, NicMode,
};

use crate::{EndpointExplorer, SiteExplorationMetrics};

/// Tracks endpoints currently being explored so periodic site exploration and ad-hoc
/// `RefreshEndpointReport` calls do not probe the same BMC at the same time.
///
/// Coordination is per-process only. `nico-api` runs a single replica, so the only window in which
/// two processes could probe the same endpoint is the brief overlap of a rolling deploy, where a
/// duplicate probe is harmless and writes are still guarded by optimistic concurrency. If `nico-api`
/// is ever scaled to multiple active replicas, this would no longer dedupe across them.
#[derive(Clone, Default)]
pub(crate) struct EndpointExplorationLocks {
    in_flight: Arc<Mutex<HashSet<IpAddr>>>,
}

/// A claim on exploring a single endpoint. The endpoint is released when this is dropped, including
/// on panic or task cancellation.
pub struct EndpointExplorationGuard {
    in_flight: Arc<Mutex<HashSet<IpAddr>>>,
    bmc_ip: IpAddr,
}

impl Drop for EndpointExplorationGuard {
    fn drop(&mut self) {
        self.in_flight
            .lock()
            .expect("EndpointExplorationLocks mutex poisoned")
            .remove(&self.bmc_ip);
    }
}

impl EndpointExplorationLocks {
    /// Try to claim exclusive exploration of `bmc_ip` within this process. Returns `None` if another
    /// task is already exploring it.
    pub(crate) fn try_claim(&self, bmc_ip: IpAddr) -> Option<EndpointExplorationGuard> {
        let claimed = self
            .in_flight
            .lock()
            .expect("EndpointExplorationLocks mutex poisoned")
            .insert(bmc_ip);

        claimed.then(|| EndpointExplorationGuard {
            in_flight: self.in_flight.clone(),
            bmc_ip,
        })
    }
}

/// Shared endpoint exploration entry point for components that need to probe BMC endpoints.
///
/// This keeps per-endpoint coordination owned by the site-explorer crate instead of exposing the
/// lock set as a separate dependency everywhere an endpoint explorer is passed.
pub struct EndpointExplorationCoordinator {
    explorer: Arc<dyn EndpointExplorer>,
    locks: EndpointExplorationLocks,
}

impl EndpointExplorationCoordinator {
    pub fn new(explorer: Arc<dyn EndpointExplorer>) -> Self {
        Self {
            explorer,
            locks: EndpointExplorationLocks::default(),
        }
    }

    /// Try to claim exclusive exploration of `bmc_ip` within this process.
    pub fn try_claim(&self, bmc_ip: IpAddr) -> Option<EndpointExplorationGuard> {
        self.locks.try_claim(bmc_ip)
    }
}

#[async_trait::async_trait]
impl EndpointExplorer for EndpointExplorationCoordinator {
    async fn explore_endpoint(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        expected: Option<&ExpectedEntity>,
        last_exploration_error: Option<&EndpointExplorationError>,
        boot_interface_mac: Option<MacAddress>,
    ) -> Result<EndpointExplorationReport, EndpointExplorationError> {
        self.explorer
            .explore_endpoint(
                address,
                interface,
                expected,
                last_exploration_error,
                boot_interface_mac,
            )
            .await
    }

    async fn check_preconditions(
        &self,
        metrics: &mut SiteExplorationMetrics,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.check_preconditions(metrics).await
    }

    async fn redfish_reset_bmc(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.redfish_reset_bmc(address, interface).await
    }

    async fn ipmitool_reset_bmc(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.ipmitool_reset_bmc(address, interface).await
    }

    async fn redfish_get_power_state(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<libredfish::PowerState, EndpointExplorationError> {
        self.explorer
            .redfish_get_power_state(address, interface)
            .await
    }

    async fn redfish_power_control(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        action: libredfish::SystemPowerControl,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer
            .redfish_power_control(address, interface, action)
            .await
    }

    async fn have_credentials(&self, interface: &MachineInterfaceSnapshot) -> bool {
        self.explorer.have_credentials(interface).await
    }

    async fn disable_secure_boot(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.disable_secure_boot(address, interface).await
    }

    async fn lockdown(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        action: libredfish::EnabledDisabled,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.lockdown(address, interface, action).await
    }

    async fn lockdown_status(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<LockdownStatus, EndpointExplorationError> {
        self.explorer.lockdown_status(address, interface).await
    }

    async fn enable_infinite_boot(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.enable_infinite_boot(address, interface).await
    }

    async fn is_infinite_boot_enabled(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<Option<bool>, EndpointExplorationError> {
        self.explorer
            .is_infinite_boot_enabled(address, interface)
            .await
    }

    async fn machine_setup(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        boot_interface: Option<&BootInterfaceTarget>,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer
            .machine_setup(address, interface, boot_interface)
            .await
    }

    async fn set_boot_order_dpu_first(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        boot_interface: &BootInterfaceTarget,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer
            .set_boot_order_dpu_first(address, interface, boot_interface)
            .await
    }

    async fn set_nic_mode(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        mode: NicMode,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.set_nic_mode(address, interface, mode).await
    }

    async fn is_viking(
        &self,
        bmc_ip_address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<bool, EndpointExplorationError> {
        self.explorer.is_viking(bmc_ip_address, interface).await
    }

    async fn clear_nvram(
        &self,
        bmc_ip_address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer.clear_nvram(bmc_ip_address, interface).await
    }

    async fn create_bmc_user(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        username: &str,
        password: &str,
        role_id: RoleId,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer
            .create_bmc_user(address, interface, username, password, role_id)
            .await
    }

    async fn delete_bmc_user(
        &self,
        address: SocketAddr,
        interface: &MachineInterfaceSnapshot,
        username: &str,
    ) -> Result<(), EndpointExplorationError> {
        self.explorer
            .delete_bmc_user(address, interface, username)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
    }

    #[test]
    fn second_claim_for_same_endpoint_is_rejected_until_released() {
        let locks = EndpointExplorationLocks::default();

        let guard = locks.try_claim(ip(1)).expect("first claim should succeed");
        assert!(
            locks.try_claim(ip(1)).is_none(),
            "second claim for the same endpoint should be rejected while held"
        );

        drop(guard);
        assert!(
            locks.try_claim(ip(1)).is_some(),
            "claim should succeed again once the previous guard is dropped"
        );
    }

    #[test]
    fn claims_are_per_endpoint() {
        let locks = EndpointExplorationLocks::default();

        let _guard_a = locks.try_claim(ip(1)).expect("claim for a should succeed");
        assert!(
            locks.try_claim(ip(2)).is_some(),
            "a claim on one endpoint must not block a different endpoint"
        );
    }
}
