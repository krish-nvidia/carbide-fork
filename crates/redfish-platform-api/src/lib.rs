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

//! Stable, capability-oriented contract that NICo state controllers use to
//! reach BMCs over Redfish.
//!
//! This crate is intentionally dependency-light and behavior-free: it defines
//! the controller-facing service traits ([`service`]), the plugin-facing
//! capability traits ([`plugin`]), the shared [`RedfishError`] (lifted from the
//! error vocabulary NICo matches on today), and the request/response DTOs
//! ([`model`]). The runtime (`carbide-redfish-platform-runtime`) and the
//! plugins (`carbide-redfish-platform-plugins`) live in separate crates so that
//! controllers depend only on this contract.
//!
//! Design reference: `docs/architecture/redfish-platform-plugins.md`.

pub mod error;
pub mod model;
pub mod ops;
pub mod plugin;
pub mod registry;
pub mod service;

pub use error::RedfishError;
pub use model::{
    BmcAccountPolicyRequest, BmcDeleteUserRequest, BmcEndpointKind, BmcPasswordRequest, BmcRef,
    BmcResetKind, BmcStatus, BmcUserRequest, BootOrderRequest, BootOrderStatus, BossController,
    CreateVolumeRequest, DecommissionRequest, DpuNicMode, DpuNicModeStatus, FirmwareComponent,
    FirmwareInventory, FirmwareUpdateRequest, JobHandle, JobKind, JobState, LockdownScope,
    LockdownStatus, MachineSetupRequest, MachineSetupStatus, MatchSpecificity, PlatformIdentity,
    PlatformMetadata, PluginId, PowerAction, PowerState, ResetTransport, SecureBootStatus,
    SelectedPlatform, SupportLevel, service_root_oem_keys,
};
pub use ops::{PlatformExecutionContext, RedfishOps};
pub use plugin::{
    BmcAccountCap, BmcResetCap, BootOrderCap, DpuCap, FirmwareCap, HostPowerCap, JobPollCap,
    LockdownCap, MachineSetupCap, PlatformPlugin, SecureBootCap, StorageCap,
};
pub use registry::PluginRegistrar;
pub use service::{
    BmcAccountOps, BmcResetOps, BootOrderOps, DpuOps, FirmwareOps, HostPowerOps, JobPollOps,
    LockdownOps, MachineSetupOps, PlatformSelection, RedfishPlatformService, SecureBootOps,
    StorageOps,
};
