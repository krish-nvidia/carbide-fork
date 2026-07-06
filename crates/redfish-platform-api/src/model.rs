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

//! Plain data types used across the contract: BMC references, the platform
//! identity used for selection, async job handles, and per-capability
//! request/response DTOs.

use std::net::SocketAddr;

use mac_address::MacAddress;
use serde::{Deserialize, Serialize};

/// Stable identifier for a plugin, e.g. `nico.redfish.hpe.ilo`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginId(pub String);

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What kind of BMC a [`BmcRef`] points at. A DPU is its own BMC, hence its own
/// endpoint kind rather than a sub-resource of the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BmcEndpointKind {
    /// Host server BMC (iDRAC/iLO/XCC/OpenBMC/etc).
    HostBmc,
    /// BlueField DPU BMC.
    DpuBmc,
    /// Power-shelf controller.
    PowerShelfBmc,
    /// Switch BMC.
    SwitchBmc,
    /// Unknown / not yet classified.
    Unknown,
}

/// BMC login credentials, passed explicitly by callers that own credential
/// selection (site exploration's credential ladder, password rotation) instead
/// of the runtime's provider lookup.
#[derive(Clone, PartialEq, Eq)]
pub struct BmcCredentials {
    /// BMC username.
    pub username: String,
    /// BMC password.
    pub password: String,
}

impl BmcCredentials {
    /// Construct credentials.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

// Manual impl: never leak the password through logs/errors.
impl std::fmt::Debug for BmcCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BmcCredentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Everything the runtime needs to resolve and reach one BMC. Carries no
/// database handles or controller state.
#[derive(Debug, Clone)]
pub struct BmcRef {
    /// Optional owning machine id (opaque string; the controller owns its
    /// meaning).
    pub machine_id: Option<String>,
    /// Optional owning site id.
    pub site_id: Option<String>,
    /// BMC network address (IP + port).
    pub address: SocketAddr,
    /// BMC MAC, used as the credential lookup key (as today).
    pub mac_address: Option<MacAddress>,
    /// Explicit login credentials. When set, the runtime uses these verbatim
    /// and skips its credential provider. Used by callers that own credential
    /// selection: site exploration's expected/factory-default ladder and
    /// password rotation (which must log in with the *current* password).
    pub credentials: Option<BmcCredentials>,
    /// What kind of BMC this is.
    pub endpoint_kind: BmcEndpointKind,
    /// Advisory plugin hint, typically the plugin id stored for this endpoint
    /// during site exploration. When set and the plugin is registered, the
    /// runtime skips live identity-gathering and uses it directly; otherwise it
    /// falls back to live identification. It is only a cache, never authority.
    pub platform_hint: Option<PluginId>,
}

impl BmcRef {
    /// Construct a minimal reference from an address; most callers will also
    /// set `mac_address` so credentials can be resolved.
    pub fn new(address: SocketAddr, endpoint_kind: BmcEndpointKind) -> Self {
        Self {
            machine_id: None,
            site_id: None,
            address,
            mac_address: None,
            credentials: None,
            endpoint_kind,
            platform_hint: None,
        }
    }

    /// Attach explicit credentials, bypassing the runtime's provider.
    pub fn with_credentials(mut self, credentials: BmcCredentials) -> Self {
        self.credentials = Some(credentials);
        self
    }
}

/// Cheap evidence used to select a plugin. Collected by the runtime from a few
/// inexpensive Redfish reads (service root, first manager, first system, and
/// chassis ids). These are exactly the signals `libredfish` and
/// `bmc-explorer::hw_type` use today.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformIdentity {
    /// `ServiceRoot.Vendor` (e.g. "Dell", "HPE", "NVIDIA", "AMI").
    pub service_root_vendor: Option<String>,
    /// `ServiceRoot.Product` (e.g. "P3809", "GB200 NVL", "GB BMC").
    pub service_root_product: Option<String>,
    /// OEM keys present under `ServiceRoot.Oem` (e.g. contains "Ami").
    pub service_root_oem_keys: Vec<String>,
    /// First manager id (e.g. "BMC").
    pub manager_id: Option<String>,
    /// Manager firmware version string.
    pub manager_firmware_version: Option<String>,
    /// First system id (e.g. "DGX", "System_0", "System.Embedded.1").
    pub system_id: Option<String>,
    /// First system manufacturer.
    pub system_manufacturer: Option<String>,
    /// First system model (e.g. contains "GB300", "SR650 V4").
    pub system_model: Option<String>,
    /// Chassis member ids (e.g. "MGX_NVSwitch_0", "Chassis_0").
    pub chassis_ids: Vec<String>,
}

/// Extract the service-root OEM keys used for plugin selection from a raw
/// `/redfish/v1` JSON document (the sorted keys of the top-level `Oem` object).
///
/// This is the single definition of "service root OEM keys", shared by the
/// runtime (live identification) and site exploration (cached hint) so the two
/// never disagree about which plugin an endpoint resolves to.
pub fn service_root_oem_keys(service_root: &serde_json::Value) -> Vec<String> {
    service_root
        .get("Oem")
        .and_then(serde_json::Value::as_object)
        .map(|oem| {
            let mut keys: Vec<String> = oem.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default()
}

/// How specific a plugin's match is. The runtime picks the most specific match;
/// the `standard` plugin returns `Generic` for everything as the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchSpecificity {
    /// Generic Redfish fallback (the `standard` plugin).
    Generic,
    /// Matched a vendor (e.g. any HPE iLO).
    Vendor,
    /// Matched a specific model/family (e.g. Viking, GB300, GH200, GB switch).
    Model,
}

/// How thoroughly a plugin has been validated. Kept lightweight: this is
/// informational metadata for logs/HCL, not a runtime policy gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportLevel {
    /// Generated/drafted from probing; dev/test only.
    Candidate,
    /// Reviewed, fixture-backed, validated on at least one real BMC.
    Validated,
    /// Maintained by NICo with CI coverage.
    CoreMaintained,
}

/// Static descriptor for a plugin.
#[derive(Debug, Clone)]
pub struct PlatformMetadata {
    /// Stable plugin id.
    pub id: PluginId,
    /// Human-readable vendor name.
    pub vendor: &'static str,
    /// Models/families this plugin targets (for docs/logging).
    pub models: &'static [&'static str],
    /// Plugin version string.
    pub plugin_version: &'static str,
    /// Validation level.
    pub support_level: SupportLevel,
}

/// Read-only result of plugin selection. Lets callers log/branch on which
/// plugin handled a BMC without exposing the plugin instance.
#[derive(Debug, Clone)]
pub struct SelectedPlatform {
    /// Selected plugin id.
    pub plugin_id: PluginId,
    /// Selected plugin version.
    pub plugin_version: String,
    /// Vendor name.
    pub vendor: String,
    /// Match specificity that won selection.
    pub specificity: MatchSpecificity,
    /// Transport a host reset must use for this platform (Redfish or IPMI).
    /// Pure plugin metadata, surfaced here so callers don't make a second call.
    pub reset_transport: ResetTransport,
    /// The identity evidence used for the decision.
    pub identity: PlatformIdentity,
}

// ---------------------------------------------------------------------------
// Async job handles
// ---------------------------------------------------------------------------

/// The kind of asynchronous work a [`JobHandle`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobKind {
    /// A standard Redfish `TaskService/Tasks/{id}` task.
    RedfishTask,
    /// A vendor job-queue item (e.g. Dell BIOS/BOSS jobs).
    VendorJob,
}

/// Opaque, serializable handle to BMC-side async work. Persisted in controller
/// DB state exactly like `set_boot_order_jid`/`task_id` today and polled via
/// [`crate::service::JobPollOps`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobHandle {
    /// Whether this is a Redfish task or a vendor job.
    pub kind: JobKind,
    /// BMC-side id; opaque to controllers.
    pub id: String,
}

impl JobHandle {
    /// Construct a Redfish task handle.
    pub fn task(id: impl Into<String>) -> Self {
        Self {
            kind: JobKind::RedfishTask,
            id: id.into(),
        }
    }

    /// Construct a vendor job handle.
    pub fn vendor_job(id: impl Into<String>) -> Self {
        Self {
            kind: JobKind::VendorJob,
            id: id.into(),
        }
    }
}

/// Current state of an async job/task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    /// Scheduled but not started.
    Pending,
    /// Running, optionally with a percent-complete hint.
    Running {
        /// Percent complete (0-100), if reported.
        percent: Option<u8>,
    },
    /// Completed successfully.
    Completed,
    /// Failed; `detail` describes why.
    Failed {
        /// Human-readable failure detail.
        detail: String,
    },
}

// ---------------------------------------------------------------------------
// Power
// ---------------------------------------------------------------------------

/// Host/BMC power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerState {
    /// Powered on.
    On,
    /// Powered off.
    Off,
    /// Other/unknown reported state.
    Unknown,
}

/// A requested power action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerAction {
    /// Power on.
    On,
    /// Graceful shutdown.
    GracefulShutdown,
    /// Hard power off.
    ForceOff,
    /// Graceful restart.
    GracefulRestart,
    /// Hard reset.
    ForceRestart,
    /// Power cycle (off-then-on as one action).
    PowerCycle,
    /// Full AC power cycle (vendor-gated).
    AcPowerCycle,
}

/// Which transport a host reset must use for the selected platform. Replaces
/// the scattered `needs_ipmi_restart()` vendor matches in machine-controller:
/// the plugin declares the requirement, the controller owns the IPMI tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetTransport {
    /// Reset via Redfish.
    Redfish,
    /// Reset must go out-of-band via IPMI (e.g. Lenovo SR650 V4, Viking).
    Ipmi,
}

// ---------------------------------------------------------------------------
// BMC reset
// ---------------------------------------------------------------------------

/// Kind of BMC reset to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BmcResetKind {
    /// Graceful manager restart.
    GracefulRestart,
    /// Force manager restart.
    ForceRestart,
    /// Reset manager to factory defaults.
    ResetToDefaults,
}

/// Reported BMC/manager status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BmcStatus {
    /// Whether the manager reports as reachable/ready.
    pub ready: bool,
    /// Manager firmware version, if known.
    pub firmware_version: Option<String>,
    /// The manager's current clock (`Manager.DateTime`, RFC 3339), if reported.
    /// Used by preingestion's NTP-drift check.
    pub date_time: Option<String>,
}

/// Request to reset a chassis sub-resource (e.g. the BlueField ERoT chassis
/// after a CEC firmware update).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChassisResetRequest {
    /// Chassis member id (e.g. "Bluefield_ERoT").
    pub chassis_id: String,
    /// Graceful or forced restart. `ResetToDefaults` is not valid for chassis.
    pub kind: BmcResetKind,
}

// ---------------------------------------------------------------------------
// Machine setup / BIOS
// ---------------------------------------------------------------------------

/// Request to apply NICo's expected machine/BIOS setup, expressed as intent.
/// The plugin owns the vendor-specific BIOS attributes; the caller toggles
/// which pieces of setup to apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSetupRequest {
    /// Enable UEFI network (HTTP/PXE) boot so NICo can network-boot the host.
    pub enable_network_boot: bool,
    /// Enable CPU virtualization extensions.
    pub enable_virtualization: bool,
    /// Configure the BMC serial console for out-of-band access.
    pub enable_serial_console: bool,
}

impl Default for MachineSetupRequest {
    fn default() -> Self {
        // NICo's standard ingestion setup applies all three.
        Self {
            enable_network_boot: true,
            enable_virtualization: true,
            enable_serial_console: true,
        }
    }
}

/// Status of machine setup convergence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSetupStatus {
    /// Whether the BMC reports the desired setup as applied.
    pub applied: bool,
    /// Whether a reboot is required to finish applying.
    pub reboot_required: bool,
}

// ---------------------------------------------------------------------------
// Boot order
// ---------------------------------------------------------------------------

/// Request to order the host to boot from its DPU NIC first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BootOrderRequest {
    /// Whether to additionally enable a one-time HTTP/PXE boot override.
    pub http_boot: bool,
}

/// Reported boot-order status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootOrderStatus {
    /// Whether the DPU/network device is first in boot order.
    pub dpu_first: bool,
    /// Whether infinite boot retry is enabled, when the platform reports it.
    pub infinite_boot: Option<bool>,
}

// ---------------------------------------------------------------------------
// Secure boot
// ---------------------------------------------------------------------------

/// Reported secure boot status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecureBootStatus {
    /// Whether secure boot is enabled.
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Lockdown
// ---------------------------------------------------------------------------

/// Which lockdown scope an operation refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockdownScope {
    /// Host-side boot/management lockdown.
    Host,
    /// BMC management-interface lockdown.
    Bmc,
}

/// Reported lockdown status across scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockdownStatus {
    /// Whether host lockdown is fully enabled.
    pub host_enabled: bool,
    /// Whether BMC-only lockdown is fully enabled.
    pub bmc_enabled: bool,
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// Request to ensure a BMC user exists with the given role/password.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmcUserRequest {
    /// Username.
    pub username: String,
    /// Password to set.
    pub password: String,
    /// Optional Redfish role id (e.g. "Administrator").
    pub role_id: Option<String>,
}

/// Request to delete a BMC user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmcDeleteUserRequest {
    /// Username to delete.
    pub username: String,
}

/// Request to change a BMC user's password.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmcPasswordRequest {
    /// Username whose password to change.
    pub username: String,
    /// New password.
    pub new_password: String,
}

/// Request to set the BMC account/password policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BmcAccountPolicyRequest {
    /// Minimum password length, if enforced.
    pub min_password_length: Option<u32>,
    /// Max failed login attempts before lockout, if enforced.
    pub max_failed_logins: Option<u32>,
}

// ---------------------------------------------------------------------------
// DPU (DPU-BMC only)
// ---------------------------------------------------------------------------

/// DPU NIC mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpuNicMode {
    /// DPU mode (BlueField runs its own OS / SmartNIC).
    Dpu,
    /// NIC mode (acts as a plain NIC).
    Nic,
}

/// Reported DPU NIC mode status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpuNicModeStatus {
    /// Current mode, if readable.
    pub mode: Option<DpuNicMode>,
}

// ---------------------------------------------------------------------------
// Firmware
// ---------------------------------------------------------------------------

/// Transfer protocol for a remote firmware image URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirmwareTransferProtocol {
    /// Plain HTTP.
    Http,
    /// HTTPS.
    Https,
}

/// Where the firmware image comes from. The plugin owns the vendor upload
/// mechanics (SimpleUpdate vs multipart vs HttpPushUri).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirmwareSource {
    /// A remote image URI the BMC pulls (Redfish `SimpleUpdate`).
    RemoteUri {
        /// Image URI.
        uri: String,
        /// Transfer protocol, when the BMC needs it stated explicitly.
        protocol: Option<FirmwareTransferProtocol>,
    },
    /// A local file NICo pushes to the BMC. The plugin picks multipart vs
    /// `HttpPushUri` per its write policy.
    LocalFile {
        /// Path to the image on the calling host.
        path: String,
    },
}

/// Request to start a firmware update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareUpdateRequest {
    /// Where the image comes from.
    pub source: FirmwareSource,
    /// Optional target component ids (vendor-specific).
    pub targets: Vec<String>,
    /// Whether to preserve existing configuration where supported.
    pub preserve_config: bool,
    /// Component being updated (e.g. "BMC", "UEFI", "CEC"), used by plugins
    /// that map components to upload targets or parameters.
    pub component_hint: Option<String>,
    /// Upload timeout override in seconds for pushed images.
    pub upload_timeout_secs: Option<u64>,
}

/// A single firmware component and its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareComponent {
    /// Component id.
    pub id: String,
    /// Reported version.
    pub version: Option<String>,
}

/// Firmware inventory readout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareInventory {
    /// Components reported by the BMC.
    pub components: Vec<FirmwareComponent>,
}

// ---------------------------------------------------------------------------
// Storage (Dell BOSS today)
// ---------------------------------------------------------------------------

/// A BOSS (or similar) storage controller summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BossController {
    /// Controller id.
    pub id: String,
    /// Number of attached drives.
    pub drive_count: u32,
}

/// Request to decommission a storage controller (secure erase).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecommissionRequest {
    /// Controller id to decommission.
    pub controller_id: String,
}

/// Request to create a storage volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVolumeRequest {
    /// Controller id to create the volume on.
    pub controller_id: String,
    /// RAID type (vendor-interpreted, e.g. "RAID1").
    pub raid_type: String,
}
