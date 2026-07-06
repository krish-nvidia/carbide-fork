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

//! Plugin-facing capability traits and the [`PlatformPlugin`] trait.
//!
//! The `*Cap` traits mirror the controller-facing `*Ops` traits one-to-one,
//! differing only in that they take a [`PlatformExecutionContext`] (the runtime
//! has already resolved credentials and selected the plugin) instead of a
//! `BmcRef`. A plugin advertises which capabilities it implements via the
//! `Option<&dyn …Cap>` accessors on [`PlatformPlugin`]; a `None` accessor means
//! the runtime returns [`RedfishError::NotSupported`].

use async_trait::async_trait;

use crate::error::RedfishError;
use crate::model::{
    BmcAccountPolicyRequest, BmcDeleteUserRequest, BmcPasswordRequest, BmcResetKind, BmcStatus,
    BmcUserRequest, BootOrderRequest, BootOrderStatus, BossController, ChassisResetRequest,
    CreateVolumeRequest, DecommissionRequest, DpuNicMode, DpuNicModeStatus, FirmwareInventory,
    FirmwareUpdateRequest, JobHandle, JobState, LockdownStatus, MachineSetupRequest,
    MachineSetupStatus, MatchSpecificity, PlatformIdentity, PlatformMetadata, PowerAction,
    PowerState, ResetTransport, SecureBootStatus,
};
use crate::ops::PlatformExecutionContext;

/// Host power capability.
#[async_trait]
pub trait HostPowerCap: Send + Sync {
    /// Read host power state.
    async fn power_state(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<PowerState, RedfishError>;

    /// Apply a power action.
    async fn set_power(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        action: PowerAction,
    ) -> Result<(), RedfishError>;

    /// Which transport a host reset must use. Defaults to Redfish.
    fn reset_transport(&self) -> ResetTransport {
        ResetTransport::Redfish
    }
}

/// BMC/manager reset capability.
#[async_trait]
pub trait BmcResetCap: Send + Sync {
    /// Read BMC/manager status.
    async fn bmc_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<BmcStatus, RedfishError>;

    /// Reset the BMC/manager.
    async fn reset_bmc(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        kind: BmcResetKind,
    ) -> Result<(), RedfishError>;

    /// Reset a chassis sub-resource. Defaults to unsupported.
    async fn reset_chassis(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: ChassisResetRequest,
    ) -> Result<(), RedfishError> {
        let _ = (ctx, req);
        Err(RedfishError::not_supported("chassis reset"))
    }

    /// Set the manager clock/timezone to UTC. Defaults to unsupported.
    async fn set_bmc_time_utc(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<(), RedfishError> {
        let _ = ctx;
        Err(RedfishError::not_supported("bmc time/timezone"))
    }
}

/// Machine/BIOS setup capability.
#[async_trait]
pub trait MachineSetupCap: Send + Sync {
    /// Apply machine/BIOS setup.
    async fn apply_machine_setup(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: MachineSetupRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Read machine-setup status.
    async fn machine_setup_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<MachineSetupStatus, RedfishError>;

    /// Set the UEFI/BIOS setup password.
    async fn set_uefi_password(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        password: String,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Clear NVRAM.
    async fn clear_nvram(&self, ctx: &PlatformExecutionContext<'_>) -> Result<(), RedfishError>;
}

/// Boot-order capability.
#[async_trait]
pub trait BootOrderCap: Send + Sync {
    /// Order the host to boot from its DPU NIC first.
    async fn set_dpu_first_boot(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BootOrderRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Read boot-order status.
    async fn boot_order_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<BootOrderStatus, RedfishError>;

    /// Enable or disable infinite boot.
    async fn set_infinite_boot(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<Option<JobHandle>, RedfishError>;
}

/// Secure-boot capability.
#[async_trait]
pub trait SecureBootCap: Send + Sync {
    /// Read secure-boot status.
    async fn secure_boot_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<SecureBootStatus, RedfishError>;

    /// Enable or disable secure boot.
    async fn set_secure_boot(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError>;

    /// Upload a secure-boot certificate.
    async fn add_certificate(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        certificate: Vec<u8>,
    ) -> Result<Option<JobHandle>, RedfishError>;
}

/// Lockdown capability.
#[async_trait]
pub trait LockdownCap: Send + Sync {
    /// Read lockdown status across scopes.
    async fn lockdown_status(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<LockdownStatus, RedfishError>;

    /// Enable or disable host lockdown.
    async fn set_host_lockdown(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError>;

    /// Enable or disable BMC-only lockdown.
    async fn set_bmc_lockdown(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError>;
}

/// BMC account capability.
#[async_trait]
pub trait BmcAccountCap: Send + Sync {
    /// Ensure a user exists.
    async fn ensure_user(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcUserRequest,
    ) -> Result<(), RedfishError>;

    /// Delete a user.
    async fn delete_user(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcDeleteUserRequest,
    ) -> Result<(), RedfishError>;

    /// Change a user's password.
    async fn change_password(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcPasswordRequest,
    ) -> Result<(), RedfishError>;

    /// Set the account/password policy.
    async fn set_account_policy(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: BmcAccountPolicyRequest,
    ) -> Result<(), RedfishError>;
}

/// DPU-BMC-only capability (NIC mode, host-rshim).
#[async_trait]
pub trait DpuCap: Send + Sync {
    /// Read DPU NIC mode.
    async fn nic_mode(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<DpuNicModeStatus, RedfishError>;

    /// Set DPU NIC mode.
    async fn set_nic_mode(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        mode: DpuNicMode,
    ) -> Result<(), RedfishError>;

    /// Enable or disable host rshim access.
    async fn set_host_rshim(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        enabled: bool,
    ) -> Result<(), RedfishError>;
}

/// Firmware capability.
#[async_trait]
pub trait FirmwareCap: Send + Sync {
    /// Start a firmware update.
    async fn start_update(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: FirmwareUpdateRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Read firmware inventory.
    async fn firmware_inventory(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<FirmwareInventory, RedfishError>;
}

/// Storage capability (Dell BOSS today).
#[async_trait]
pub trait StorageCap: Send + Sync {
    /// Read the BOSS (or similar) controller, if present.
    async fn boss_controller(
        &self,
        ctx: &PlatformExecutionContext<'_>,
    ) -> Result<Option<BossController>, RedfishError>;

    /// Decommission a storage controller.
    async fn decommission(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: DecommissionRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;

    /// Create a storage volume.
    async fn create_volume(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        req: CreateVolumeRequest,
    ) -> Result<Option<JobHandle>, RedfishError>;
}

/// Polling capability. A plugin that issues async jobs (returns a [`JobHandle`]
/// from any mutation) must implement this to interpret its own handles. If a
/// plugin returns a handle but does not implement `job_poll`, the runtime
/// surfaces `NotSupported` (there is no built-in Redfish task poller yet).
#[async_trait]
pub trait JobPollCap: Send + Sync {
    /// Poll a job/task.
    async fn poll(
        &self,
        ctx: &PlatformExecutionContext<'_>,
        job: &JobHandle,
    ) -> Result<JobState, RedfishError>;
}

/// A platform plugin: detection plus the capabilities it implements.
///
/// Capabilities default to `None` (unsupported). A plugin overrides only the
/// accessors it implements; the runtime maps a `None` accessor to
/// [`RedfishError::NotSupported`].
pub trait PlatformPlugin: Send + Sync {
    /// Static descriptor (id, vendor, models, version, support level).
    fn metadata(&self) -> &PlatformMetadata;

    /// Decide whether (and how specifically) this plugin handles the identity.
    /// Pure and synchronous: no I/O. Returns `None` for no match.
    fn detect(&self, identity: &PlatformIdentity) -> Option<MatchSpecificity>;

    /// Host power capability.
    fn power(&self) -> Option<&dyn HostPowerCap> {
        None
    }
    /// BMC reset capability.
    fn bmc_reset(&self) -> Option<&dyn BmcResetCap> {
        None
    }
    /// Machine setup capability.
    fn machine_setup(&self) -> Option<&dyn MachineSetupCap> {
        None
    }
    /// Boot order capability.
    fn boot_order(&self) -> Option<&dyn BootOrderCap> {
        None
    }
    /// Secure boot capability.
    fn secure_boot(&self) -> Option<&dyn SecureBootCap> {
        None
    }
    /// Lockdown capability.
    fn lockdown(&self) -> Option<&dyn LockdownCap> {
        None
    }
    /// Account capability.
    fn accounts(&self) -> Option<&dyn BmcAccountCap> {
        None
    }
    /// DPU capability (DPU-BMC plugins only).
    fn dpu(&self) -> Option<&dyn DpuCap> {
        None
    }
    /// Firmware capability.
    fn firmware(&self) -> Option<&dyn FirmwareCap> {
        None
    }
    /// Storage capability.
    fn storage(&self) -> Option<&dyn StorageCap> {
        None
    }
    /// Custom job-poll capability. If `None`, the runtime polls Redfish tasks
    /// with its standard poller.
    fn job_poll(&self) -> Option<&dyn JobPollCap> {
        None
    }
}
