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

//! Controller-facing service traits, one per capability group.
//!
//! Handlers depend only on the sub-trait they use (e.g. `Arc<dyn HostPowerOps>`)
//! so their mocks stay small. All methods are keyed by [`BmcRef`]; the runtime
//! resolves credentials, builds the session, selects the plugin, and dispatches
//! per call. The umbrella [`RedfishPlatformService`] composes every sub-trait.

use async_trait::async_trait;

use std::net::SocketAddr;

use crate::error::RedfishError;
use crate::model::{
    BmcAccountPolicyRequest, BmcDeleteUserRequest, BmcPasswordRequest, BmcRef, BmcResetKind,
    BmcStatus, BmcUserRequest, BootOrderRequest, BootOrderStatus, BossController,
    ChassisResetRequest, CreateVolumeRequest, DecommissionRequest, DpuNicMode, DpuNicModeStatus,
    FirmwareInventory, FirmwareUpdateRequest, JobHandle, JobState, LockdownStatus,
    MachineSetupRequest, MachineSetupStatus, PowerAction, PowerState, SecureBootStatus,
    SelectedPlatform,
};

/// Plugin selection / introspection.
#[async_trait]
pub trait PlatformSelection: Send + Sync {
    /// Resolve which plugin handles this BMC (read-only; for logging/branching).
    async fn selected_platform(&self, bmc: BmcRef) -> Result<SelectedPlatform, RedfishError>;

    /// Anonymous reachability probe: is a Redfish service root responding at
    /// this address? Needs no credentials; used to wait out BMC reboots (e.g.
    /// a DPU BMC coming back during BFB recovery).
    async fn probe_endpoint(&self, address: SocketAddr) -> Result<(), RedfishError>;
}

/// Host power control.
#[async_trait]
pub trait HostPowerOps: Send + Sync {
    /// Read host power state.
    async fn power_state(&self, bmc: BmcRef) -> Result<PowerState, RedfishError>;

    /// Apply a power action.
    ///
    /// Whether a host reset must instead go out-of-band via IPMI is exposed as
    /// [`SelectedPlatform::reset_transport`] (from [`PlatformSelection`]), so
    /// the controller reads it without a second call.
    async fn set_power(&self, bmc: BmcRef, action: PowerAction) -> Result<(), RedfishError>;
}

/// BMC/manager reset.
#[async_trait]
pub trait BmcResetOps: Send + Sync {
    /// Read BMC/manager status.
    async fn bmc_status(&self, bmc: BmcRef) -> Result<BmcStatus, RedfishError>;

    /// Reset the BMC/manager.
    async fn reset_bmc(&self, bmc: BmcRef, kind: BmcResetKind) -> Result<(), RedfishError>;

    /// Reset a chassis sub-resource (e.g. the BlueField ERoT after a CEC
    /// firmware update).
    async fn reset_chassis(&self, bmc: BmcRef, req: ChassisResetRequest)
    -> Result<(), RedfishError>;

    /// Set the manager clock/timezone to UTC (preingestion time-sync).
    async fn set_bmc_time_utc(&self, bmc: BmcRef) -> Result<(), RedfishError>;
}

/// Machine/BIOS setup, NVRAM clear, and the UEFI/BIOS setup password.
#[async_trait]
pub trait MachineSetupOps: Send + Sync {
    /// Apply NICo's expected machine/BIOS setup.
    async fn apply_machine_setup(
        &self,
        bmc: BmcRef,
        req: MachineSetupRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Read machine-setup convergence status.
    async fn machine_setup_status(&self, bmc: BmcRef) -> Result<MachineSetupStatus, RedfishError>;

    /// Set the UEFI/BIOS setup password.
    async fn set_uefi_password(
        &self,
        bmc: BmcRef,
        password: String,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Clear NVRAM.
    async fn clear_nvram(&self, bmc: BmcRef) -> Result<(), RedfishError>;
}

/// Boot-order control.
#[async_trait]
pub trait BootOrderOps: Send + Sync {
    /// Order the host to boot from its DPU NIC first (a host-BMC op; `NoDpu`
    /// surfaces on DPU-less hosts).
    async fn set_dpu_first_boot(
        &self,
        bmc: BmcRef,
        req: BootOrderRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Read boot-order status.
    async fn boot_order_status(&self, bmc: BmcRef) -> Result<BootOrderStatus, RedfishError>;

    /// Enable or disable infinite boot.
    async fn set_infinite_boot(
        &self,
        bmc: BmcRef,
        enabled: bool,
    ) -> Result<Option<JobHandle>, RedfishError>;
}

/// Secure boot control.
#[async_trait]
pub trait SecureBootOps: Send + Sync {
    /// Read secure-boot status.
    async fn secure_boot_status(&self, bmc: BmcRef) -> Result<SecureBootStatus, RedfishError>;

    /// Enable or disable secure boot.
    async fn set_secure_boot(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError>;

    /// Upload a secure-boot certificate (PEM/DER bytes).
    async fn add_certificate(
        &self,
        bmc: BmcRef,
        certificate: Vec<u8>,
    ) -> Result<Option<JobHandle>, RedfishError>;
}

/// Lockdown control. Host and BMC scopes are separate knobs.
#[async_trait]
pub trait LockdownOps: Send + Sync {
    /// Read lockdown status across scopes.
    async fn lockdown_status(&self, bmc: BmcRef) -> Result<LockdownStatus, RedfishError>;

    /// Enable or disable host lockdown.
    async fn set_host_lockdown(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError>;

    /// Enable or disable BMC-only lockdown.
    async fn set_bmc_lockdown(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError>;
}

/// BMC account management.
#[async_trait]
pub trait BmcAccountOps: Send + Sync {
    /// Ensure a user exists with the given role/password.
    async fn ensure_user(&self, bmc: BmcRef, req: BmcUserRequest) -> Result<(), RedfishError>;

    /// Delete a user.
    async fn delete_user(&self, bmc: BmcRef, req: BmcDeleteUserRequest)
    -> Result<(), RedfishError>;

    /// Change a user's password.
    async fn change_password(
        &self,
        bmc: BmcRef,
        req: BmcPasswordRequest,
    ) -> Result<(), RedfishError>;

    /// Set the account/password policy.
    async fn set_account_policy(
        &self,
        bmc: BmcRef,
        req: BmcAccountPolicyRequest,
    ) -> Result<(), RedfishError>;
}

/// DPU-BMC-only operations (NIC mode, host-rshim). Implemented only by DPU
/// plugins and invoked against a DPU's own [`BmcRef`].
#[async_trait]
pub trait DpuOps: Send + Sync {
    /// Read DPU NIC mode.
    async fn nic_mode(&self, bmc: BmcRef) -> Result<DpuNicModeStatus, RedfishError>;

    /// Set DPU NIC mode.
    async fn set_nic_mode(&self, bmc: BmcRef, mode: DpuNicMode) -> Result<(), RedfishError>;

    /// Enable or disable host rshim access.
    async fn set_host_rshim(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError>;
}

/// Firmware update + inventory.
#[async_trait]
pub trait FirmwareOps: Send + Sync {
    /// Start a firmware update.
    async fn start_update(
        &self,
        bmc: BmcRef,
        req: FirmwareUpdateRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Read firmware inventory.
    async fn firmware_inventory(&self, bmc: BmcRef) -> Result<FirmwareInventory, RedfishError>;
}

/// Storage controller operations (Dell BOSS today).
#[async_trait]
pub trait StorageOps: Send + Sync {
    /// Read the BOSS (or similar) controller, if present.
    async fn boss_controller(&self, bmc: BmcRef) -> Result<Option<BossController>, RedfishError>;

    /// Decommission (secure-erase) a storage controller.
    async fn decommission(
        &self,
        bmc: BmcRef,
        req: DecommissionRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Create a storage volume.
    async fn create_volume(
        &self,
        bmc: BmcRef,
        req: CreateVolumeRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;
}

/// Polling for any capability that returned a [`JobHandle`].
#[async_trait]
pub trait JobPollOps: Send + Sync {
    /// Poll an async job/task.
    async fn poll(&self, bmc: BmcRef, job: &JobHandle) -> Result<JobState, RedfishError>;
}

/// Umbrella trait composing every capability for callers that want the whole
/// surface. With trait upcasting (stable since Rust 1.86) an
/// `Arc<dyn RedfishPlatformService>` narrows to any single sub-trait.
pub trait RedfishPlatformService:
    PlatformSelection
    + HostPowerOps
    + BmcResetOps
    + MachineSetupOps
    + BootOrderOps
    + SecureBootOps
    + LockdownOps
    + BmcAccountOps
    + DpuOps
    + FirmwareOps
    + StorageOps
    + JobPollOps
{
}
