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
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use carbide_network::deserialize_input_mac_to_address;
use carbide_redfish::libredfish::conv::{IntoModel, bmc_vendor};
use carbide_redfish::libredfish::dpu_bios::is_dpu_bios_attributes_not_ready;
use carbide_redfish::libredfish::{RedfishAuth, RedfishClientCreationError, RedfishClientPool};
use carbide_redfish::nv_redfish::{NvRedfishClientPool, ServiceRoot};
use carbide_redfish_platform_api::RedfishError as PlatformRedfishError;
use carbide_redfish_platform_api::model::{
    BmcAccountPolicyRequest, BmcCredentials, BmcDeleteUserRequest, BmcEndpointKind,
    BmcPasswordRequest, BmcRef, BmcResetKind, BmcUserRequest, BootOrderRequest, DpuNicMode,
    MachineSetupRequest, PlatformIdentity, PowerAction,
};
use carbide_redfish_platform_api::service::RedfishPlatformService;
use carbide_secrets::credentials::Credentials;
use libredfish::model::service_root::RedfishVendor;
use libredfish::{BootInterfaceRef, Redfish, RedfishError};
use mac_address::MacAddress;
use model::site_explorer::{
    BootOption, BootOrder, Chassis, ComputerSystem, ComputerSystemAttributes,
    EndpointExplorationError, EndpointExplorationReport, EndpointType, EthernetInterface,
    InternalLockdownStatus, Inventory, LockdownStatus, MachineSetupDiff, MachineSetupStatus,
    Manager, NetworkAdapter, NicMode, PCIeDevice, SecureBootStatus, Service, UefiDevicePath,
};
use regex::Regex;

const NOT_FOUND: u16 = 404;

/// Select the Redfish platform plugin id for an explored endpoint from the same
/// evidence the runtime uses, built from the service root plus the report we
/// just generated (no extra BMC reads). Returns `None` when no plugin matches.
fn select_platform_plugin_id(
    service_root: &ServiceRoot,
    report: &EndpointExplorationReport,
    oem_keys: Vec<String>,
) -> Option<String> {
    let first_system = report.systems.first();
    let identity = PlatformIdentity {
        service_root_vendor: service_root.vendor().map(|v| v.to_string()),
        service_root_product: service_root.product().map(|p| p.to_string()),
        service_root_oem_keys: oem_keys,
        manager_id: report.managers.first().map(|m| m.id.clone()),
        manager_firmware_version: None,
        system_id: first_system.map(|s| s.id.clone()),
        system_manufacturer: first_system.and_then(|s| s.manufacturer.clone()),
        system_model: first_system
            .and_then(|s| s.model.clone())
            .or_else(|| report.model.clone()),
        chassis_ids: report.chassis.iter().map(|c| c.id.clone()).collect(),
    };
    carbide_redfish_platform_runtime::select_plugin_id(&identity).map(|id| id.0)
}

// RedfishClient is a wrapper around a redfish client pool and implements redfish utility functions that the site explorer utilizes.
// TODO: In the future, we should refactor a lot of this client's work to api/src/redfish.rs because other components in carbide can utilize this functionality.
// Eventually, this file should only have code related to generating the site exploration report.
pub struct RedfishClient {
    // Mutations (power, lockdown, accounts, ...) go through the platform
    // service; the legacy pools below remain only for read-only exploration
    // and report generation.
    platform: Arc<dyn RedfishPlatformService>,
    redfish_client_pool: Arc<dyn RedfishClientPool>,
    nv_redfish_client_pool: Arc<NvRedfishClientPool>,
}

impl RedfishClient {
    pub fn new(
        platform: Arc<dyn RedfishPlatformService>,
        redfish_client_pool: Arc<dyn RedfishClientPool>,
        nv_redfish_client_pool: Arc<NvRedfishClientPool>,
    ) -> Self {
        Self {
            platform,
            redfish_client_pool,
            nv_redfish_client_pool,
        }
    }

    /// Build a [`BmcRef`] for a platform-service call, attaching the explicit
    /// credentials site-explorer selected (it owns the vault/factory credential
    /// ladder, so the runtime's credential provider is always bypassed).
    fn bmc_ref(bmc_ip_address: SocketAddr, credentials: &Credentials) -> BmcRef {
        let Credentials::UsernamePassword { username, password } = credentials;
        BmcRef::new(bmc_ip_address, BmcEndpointKind::Unknown)
            .with_credentials(BmcCredentials::new(username.clone(), password.clone()))
    }

    async fn create_redfish_client(
        &self,
        bmc_ip_address: SocketAddr,
        auth: RedfishAuth,
        vendor: Option<RedfishVendor>,
    ) -> Result<Box<dyn Redfish>, RedfishClientCreationError> {
        self.redfish_client_pool
            .create_client(
                &bmc_ip_address.ip().to_string(),
                Some(bmc_ip_address.port()),
                auth,
                vendor,
            )
            .await
    }

    async fn create_anon_redfish_client(
        &self,
        bmc_ip_address: SocketAddr,
    ) -> Result<Box<dyn Redfish>, RedfishClientCreationError> {
        // This currently uses a "standard" client without any vendor
        // specific implementations. If we end up ever needing vendor
        // specific support for a caller using this, we could simply
        // just drop in vendor: Option<RedfishVendor> to support an
        // override of using RedfishVendor::Unknown.
        self.create_redfish_client(
            bmc_ip_address,
            RedfishAuth::Anonymous,
            Some(RedfishVendor::Unknown),
        )
        .await
    }

    pub async fn create_direct_redfish_client(
        &self,
        bmc_ip_address: SocketAddr,
        Credentials::UsernamePassword { username, password }: Credentials,
        vendor: Option<RedfishVendor>,
    ) -> Result<Box<dyn Redfish>, RedfishClientCreationError> {
        self.create_redfish_client(
            bmc_ip_address,
            RedfishAuth::Direct(username, password),
            vendor,
        )
        .await
    }

    async fn create_authenticated_redfish_client(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<Box<dyn Redfish>, RedfishClientCreationError> {
        self.create_direct_redfish_client(bmc_ip_address, credentials, None)
            .await
    }

    pub async fn get_redfish_vendor(
        &self,
        bmc_ip_address: SocketAddr,
    ) -> Result<RedfishVendor, EndpointExplorationError> {
        let client = self
            .create_anon_redfish_client(bmc_ip_address)
            .await
            .map_err(map_redfish_client_creation_error)?;

        let service_root = client.get_service_root().await.map_err(map_redfish_error)?;

        if service_root.vendor.is_none() {
            return Err(EndpointExplorationError::MissingVendor);
        }

        let Some(vendor) = service_root.vendor() else {
            tracing::info!("No vendor found for BMC at {bmc_ip_address}");
            return Err(EndpointExplorationError::MissingVendor);
        };

        Ok(vendor)
    }

    pub async fn validate_bmc_credentials(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        let client = self
            .create_direct_redfish_client(bmc_ip_address, credentials, Some(RedfishVendor::Unknown))
            .await
            .map_err(map_redfish_client_creation_error)?;

        client.get_systems().await.map_err(map_redfish_error)?;

        Ok(())
    }

    pub async fn set_bmc_root_password(
        &self,
        bmc_ip_address: SocketAddr,
        vendor: RedfishVendor,
        current_bmc_root_credentials: Credentials,
        new_password: String,
    ) -> Result<(), EndpointExplorationError> {
        let Credentials::UsernamePassword {
            username: curr_user,
            password: curr_password,
        } = &current_bmc_root_credentials;

        // Rotate the password with the CURRENT credentials attached. Which
        // account gets PATCHed (by id "1"/"2", by current user, by name) is
        // plugin policy now. The platform runtime gathers identity from the
        // service root only and tolerates HTTP 403 PasswordChangeRequired on
        // /Systems, so factory-state BMCs (e.g. NVIDIA GBx00) still select a
        // plugin -- this replaces the old deliberate `RedfishVendor::Unknown`
        // uninitialized-client trick that skipped libredfish's full init.
        let bmc = Self::bmc_ref(bmc_ip_address, &current_bmc_root_credentials);
        self.platform
            .change_password(
                bmc,
                BmcPasswordRequest {
                    username: curr_user.clone(),
                    new_password: new_password.clone(),
                },
            )
            .await
            .map_err(|err| redact_platform_password(err, new_password.as_str()))
            .map_err(|err| redact_platform_password(err, curr_password.as_str()))
            .map_err(|err| {
                tracing::error!(
                    "Failed to rotate BMC password for vendor {:?} (bmc_ip = {}): {:?}",
                    vendor,
                    bmc_ip_address,
                    err
                );
                map_platform_error(err)
            })?;

        // Log in using the new credentials and set the account/password policy
        // (mirrors the legacy `set_machine_password_policy`). The legacy impls
        // only tuned vendor-specific `AccountLockout*` knobs, whose values the
        // plugin owns now, so neither policy field is pinned here.
        let new_credentials = Credentials::UsernamePassword {
            username: curr_user.to_string(),
            password: new_password.clone(),
        };
        let bmc = Self::bmc_ref(bmc_ip_address, &new_credentials);
        self.platform
            .set_account_policy(bmc, BmcAccountPolicyRequest::default())
            .await
            .map_err(|err| redact_platform_password(err, new_password.as_str()))
            .map_err(map_platform_error)?;

        Ok(())
    }

    pub async fn generate_exploration_report(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        boot_interface_mac: Option<MacAddress>,
        vendor: Option<RedfishVendor>,
    ) -> Result<EndpointExplorationReport, EndpointExplorationError> {
        let client = self
            .create_direct_redfish_client(bmc_ip_address, credentials, vendor)
            .await
            .map_err(map_redfish_client_creation_error)?;

        let service_root = client.get_service_root().await.map_err(map_redfish_error)?;
        let vendor = service_root.vendor().map(bmc_vendor);

        let manager = fetch_manager(client.as_ref())
            .await
            .map_err(map_redfish_error)?;
        let system = fetch_system(client.as_ref()).await?;

        // TODO (spyda): once we test the BMC reset logic, we can enhance our logic here
        // to detect cases where the host's BMC is returning invalid (empty) chassis information, even though
        // an error is not returned.
        let chassis = fetch_chassis(client.as_ref())
            .await
            .map_err(map_redfish_error)?;
        let service = fetch_service(client.as_ref())
            .await
            .map_err(map_redfish_error)?;
        let is_dpu = system.id.to_lowercase().contains("bluefield");
        let (machine_setup_status, remediation_error) = match fetch_machine_setup_status(
            client.as_ref(),
            boot_interface_mac,
        )
        .await
        {
            Ok(status) => (Some(status), None),
            Err(error) if is_dpu && is_dpu_bios_attributes_not_ready(&error) => {
                let details = format!(
                    "DPU BMC BIOS attributes not ready ({error}); scheduling a force-restart to mitigate the known UEFI POST/BMC race"
                );
                tracing::warn!("{details}");
                (
                    None,
                    Some(EndpointExplorationError::InvalidDpuRedfishBiosResponse {
                        details,
                        response_body: None,
                        response_code: None,
                    }),
                )
            }
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch machine setup status.");
                (None, None)
            }
        };

        let secure_boot_status = fetch_secure_boot_status(client.as_ref())
            .await
            .inspect_err(
                |error| tracing::warn!(%error, "Failed to fetch forge secure boot status."),
            )
            .ok();

        let lockdown_status = fetch_lockdown_status(client.as_ref())
            .await
            .inspect_err(|error| {
                if !matches!(error, libredfish::RedfishError::NotSupported(_)) {
                    tracing::warn!(%error, "Failed to fetch lockdown status.");
                }
            })
            .ok();

        Ok(EndpointExplorationReport {
            endpoint_type: EndpointType::Bmc,
            last_exploration_error: None,
            last_exploration_latency: None,
            machine_id: None,
            managers: vec![manager],
            systems: vec![system],
            chassis,
            service,
            vendor,
            versions: HashMap::default(),
            model: None,
            // Only the nv-redfish path computes the platform plugin hint today.
            platform_plugin_id: None,
            power_shelf_id: None,
            switch_id: None,
            machine_setup_status,
            secure_boot_status,
            lockdown_status,
            physical_slot_number: None,
            compute_tray_index: None,
            topology_id: None,
            revision_id: None,
            remediation_error,
        })
    }

    pub async fn nv_generate_exploration_report(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        boot_interface_mac: Option<MacAddress>,
    ) -> Result<EndpointExplorationReport, EndpointExplorationError> {
        let service_root = self
            .nv_redfish_client_pool
            .service_root(bmc_ip_address, credentials.clone())
            .await
            .map_err(|err| EndpointExplorationError::Other {
                details: format!("Cannot Redfish service root: {err}"),
            })?;

        let mut report = bmc_explorer::nv_generate_exploration_report(
            service_root.clone(),
            &nv_bmc_explore_config(boot_interface_mac),
        )
        .await
        .map_err(map_nv_redfish_explore_error)?;

        // OEM keys are a primary selection signal but the typed service root
        // exposes no generic OEM list, so read them from the raw service root
        // (best-effort; failure just yields no OEM signal).
        let oem_keys = self
            .nv_redfish_client_pool
            .service_root_oem_keys(bmc_ip_address, credentials)
            .await
            .unwrap_or_default();

        // Cache the platform plugin this endpoint resolves to, using the same
        // registry selection the runtime uses, so controllers can later pass it
        // as a `BmcRef` hint and skip live re-identification.
        report.platform_plugin_id = select_platform_plugin_id(&service_root, &report, oem_keys);

        Ok(report)
    }

    pub async fn reset_bmc(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        self.platform
            .reset_bmc(
                Self::bmc_ref(bmc_ip_address, &credentials),
                BmcResetKind::GracefulRestart,
            )
            .await
            .map_err(map_platform_error)
    }

    pub async fn get_power_state(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<libredfish::PowerState, EndpointExplorationError> {
        let state = self
            .platform
            .power_state(Self::bmc_ref(bmc_ip_address, &credentials))
            .await
            .map_err(map_platform_error)?;

        Ok(match state {
            carbide_redfish_platform_api::model::PowerState::On => libredfish::PowerState::On,
            carbide_redfish_platform_api::model::PowerState::Off => libredfish::PowerState::Off,
            carbide_redfish_platform_api::model::PowerState::Unknown => {
                libredfish::PowerState::Unknown
            }
        })
    }

    pub async fn power(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        action: libredfish::SystemPowerControl,
    ) -> Result<(), EndpointExplorationError> {
        let action = match action {
            libredfish::SystemPowerControl::On => PowerAction::On,
            libredfish::SystemPowerControl::GracefulShutdown => PowerAction::GracefulShutdown,
            libredfish::SystemPowerControl::ForceOff => PowerAction::ForceOff,
            libredfish::SystemPowerControl::GracefulRestart => PowerAction::GracefulRestart,
            libredfish::SystemPowerControl::ForceRestart => PowerAction::ForceRestart,
            libredfish::SystemPowerControl::PowerCycle => PowerAction::PowerCycle,
            libredfish::SystemPowerControl::ACPowercycle => PowerAction::AcPowerCycle,
        };

        self.platform
            .set_power(Self::bmc_ref(bmc_ip_address, &credentials), action)
            .await
            .map_err(map_platform_error)
    }

    pub async fn disable_secure_boot(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        self.platform
            .set_secure_boot(Self::bmc_ref(bmc_ip_address, &credentials), false)
            .await
            .map_err(map_platform_error)
    }

    pub async fn lockdown(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        action: libredfish::EnabledDisabled,
    ) -> Result<(), EndpointExplorationError> {
        // The legacy full-platform lockdown covers both scopes: host and BMC.
        let enabled = action.is_enabled();

        self.platform
            .set_host_lockdown(Self::bmc_ref(bmc_ip_address, &credentials), enabled)
            .await
            .map_err(map_platform_error)?;
        self.platform
            .set_bmc_lockdown(Self::bmc_ref(bmc_ip_address, &credentials), enabled)
            .await
            .map_err(map_platform_error)?;

        Ok(())
    }

    pub async fn lockdown_status(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<LockdownStatus, EndpointExplorationError> {
        let status = self
            .platform
            .lockdown_status(Self::bmc_ref(bmc_ip_address, &credentials))
            .await
            .map_err(map_platform_error)?;

        // Derive the legacy classification: both scopes locked = Enabled,
        // neither = Disabled, mixed = Partial.
        let internal_status = match (status.host_enabled, status.bmc_enabled) {
            (true, true) => InternalLockdownStatus::Enabled,
            (false, false) => InternalLockdownStatus::Disabled,
            _ => InternalLockdownStatus::Partial,
        };

        Ok(LockdownStatus {
            status: internal_status,
            message: format!(
                "host lockdown enabled: {}, BMC lockdown enabled: {}",
                status.host_enabled, status.bmc_enabled
            ),
        })
    }

    pub async fn enable_infinite_boot(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        self.platform
            .set_infinite_boot(Self::bmc_ref(bmc_ip_address, &credentials), true)
            .await
            .map(|_job| ())
            .map_err(map_platform_error)
    }

    pub async fn is_infinite_boot_enabled(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<Option<bool>, EndpointExplorationError> {
        let status = self
            .platform
            .boot_order_status(Self::bmc_ref(bmc_ip_address, &credentials))
            .await
            .map_err(map_platform_error)?;

        // `None` = the platform doesn't report infinite boot, matching the
        // legacy `Ok(None)` on unsupported platforms.
        Ok(status.infinite_boot)
    }

    pub async fn machine_setup(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        // The plugin owns boot-interface resolution and the vendor BIOS
        // attributes, so the legacy boot-interface and BIOS-profile arguments
        // are gone.
        self.platform
            .apply_machine_setup(
                Self::bmc_ref(bmc_ip_address, &credentials),
                MachineSetupRequest::default(),
            )
            .await
            .map(|_job| ())
            .map_err(map_platform_error)
    }

    pub async fn set_boot_order_dpu_first(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        // The plugin resolves the DPU NIC; no boot-interface argument needed.
        self.platform
            .set_dpu_first_boot(
                Self::bmc_ref(bmc_ip_address, &credentials),
                BootOrderRequest { http_boot: false },
            )
            .await
            .map(|_job| ())
            .map_err(map_platform_error)
    }

    pub async fn set_nic_mode(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        mode: NicMode,
    ) -> Result<(), EndpointExplorationError> {
        let mode = match mode {
            NicMode::Dpu => DpuNicMode::Dpu,
            NicMode::Nic => DpuNicMode::Nic,
        };

        self.platform
            .set_nic_mode(Self::bmc_ref(bmc_ip_address, &credentials), mode)
            .await
            .map_err(map_platform_error)
    }

    pub async fn is_viking(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<bool, EndpointExplorationError> {
        let selected = self
            .platform
            .selected_platform(Self::bmc_ref(bmc_ip_address, &credentials))
            .await
            .map_err(map_platform_error)?;

        // Same predicate the legacy path computed from the service root,
        // system, and manager: an AMI BMC on a "DGX" system.
        let identity = &selected.identity;
        Ok(identity
            .service_root_vendor
            .as_deref()
            .is_some_and(|vendor| vendor.eq_ignore_ascii_case("ami"))
            && identity.system_id.as_deref() == Some("DGX")
            && identity.manager_id.as_deref() == Some("BMC"))
    }

    pub async fn clear_nvram(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
    ) -> Result<(), EndpointExplorationError> {
        self.platform
            .clear_nvram(Self::bmc_ref(bmc_ip_address, &credentials))
            .await
            .map_err(map_platform_error)
    }

    pub async fn create_bmc_user(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        new_username: &str,
        new_password: &str,
        new_user_role_id: libredfish::RoleId,
    ) -> Result<(), EndpointExplorationError> {
        self.platform
            .ensure_user(
                Self::bmc_ref(bmc_ip_address, &credentials),
                BmcUserRequest {
                    username: new_username.to_string(),
                    password: new_password.to_string(),
                    role_id: Some(new_user_role_id.to_string()),
                },
            )
            .await
            .map_err(map_platform_error)
    }

    pub async fn delete_bmc_user(
        &self,
        bmc_ip_address: SocketAddr,
        credentials: Credentials,
        delete_user: &str,
    ) -> Result<(), EndpointExplorationError> {
        self.platform
            .delete_user(
                Self::bmc_ref(bmc_ip_address, &credentials),
                BmcDeleteUserRequest {
                    username: delete_user.to_string(),
                },
            )
            .await
            .map_err(map_platform_error)
    }

    pub async fn probe_vendor_name_from_chassis(
        &self,
        bmc_ip_address: SocketAddr,
        username: String,
        password: String,
    ) -> Result<String, EndpointExplorationError> {
        let client = self
            .create_authenticated_redfish_client(
                bmc_ip_address,
                Credentials::UsernamePassword { username, password },
            )
            .await
            .map_err(map_redfish_client_creation_error)?;

        let chassis_ids = client.get_chassis_all().await.map_err(map_redfish_error)?;
        for chassis_id in &chassis_ids {
            let chassis = client
                .get_chassis(chassis_id)
                .await
                .map_err(map_redfish_error)?;
            if let Some(manufacturer) = chassis.manufacturer {
                return Ok(manufacturer);
            }
        }

        Err(EndpointExplorationError::UnsupportedVendor {
            vendor: "Unknown".to_string(),
        })
    }
}

async fn is_switch(client: &dyn Redfish) -> Result<bool, RedfishError> {
    let chassis = client.get_chassis_all().await?;
    Ok(chassis.contains(&"MGX_NVSwitch_0".to_string()))
}

async fn is_powershelf(client: &dyn Redfish) -> Result<bool, RedfishError> {
    let chassis_ids = client.get_chassis_all().await?;
    for chassis_id in &chassis_ids {
        if chassis_id == "powershelf" {
            return Ok(true);
        }
        if let Ok(chassis) = client.get_chassis(chassis_id).await
            && chassis.manufacturer.as_ref().is_some_and(|m| {
                let m = m.to_lowercase();
                m.contains("lite-on") || m.contains("delta")
            })
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn fetch_manager(client: &dyn Redfish) -> Result<Manager, RedfishError> {
    let manager = client.get_manager().await?;
    let ethernet_interfaces = fetch_ethernet_interfaces(client, false, false)
        .await
        .or_else(|err| match err {
            RedfishError::NotSupported(_) => Ok(vec![]),
            _ => Err(err),
        })?;

    Ok(Manager {
        ethernet_interfaces,
        id: manager.id,
    })
}

async fn fetch_system(client: &dyn Redfish) -> Result<ComputerSystem, EndpointExplorationError> {
    let mut system = client.get_system().await.map_err(map_redfish_error)?;
    let is_dpu = system.id.to_lowercase().contains("bluefield");
    let ethernet_interfaces = match fetch_ethernet_interfaces(client, true, is_dpu).await {
        Ok(interfaces) => Ok(interfaces),
        Err(e) if is_dpu => {
            tracing::warn!(
                "Error getting system ethernet interfaces.  The error will be ignored. ({e})"
            );
            Ok(Vec::default())
        }
        Err(e) => Err(map_redfish_error(e)),
    }?;
    let mut base_mac = None;
    let mut nic_mode = None;

    let is_switch = is_switch(client).await.map_err(map_redfish_error)?;
    let is_powershelf = is_powershelf(client).await.map_err(map_redfish_error)?;
    if is_dpu {
        // This part processes dpu case and do two things such as
        // 1. update system serial_number in case it is empty using chassis serial_number
        // 2. format serial_number data using the same rules as in fetch_chassis()
        if system.serial_number.is_none() {
            let chassis = client
                .get_chassis("Card1")
                .await
                .map_err(map_redfish_error)?;
            system.serial_number = chassis.serial_number;
        }

        base_mac = match client.get_base_mac_address().await {
            Ok(base_mac) => base_mac.and_then(|v| {
                v.parse()
                    .inspect_err(|err| {
                        tracing::warn!("Failed to parse BaseMAC: {err} (mac: {v})");
                    })
                    .ok()
            }),
            Err(error) => {
                tracing::info!(
                    "Could not use new method to retreive base mac address for DPU (serial number {:#?}): {error}",
                    system.serial_number
                );
                None
            }
        };

        nic_mode = match client.get_nic_mode().await {
            Ok(nic_mode) => nic_mode,
            Err(e) => return Err(map_redfish_error(e)),
        };
    }

    system.serial_number = system.serial_number.map(|s| s.trim().to_string());

    let pcie_devices = if !is_powershelf {
        fetch_pcie_devices(client)
            .await
            .map_err(map_redfish_error)?
    } else {
        vec![]
    };

    let is_infinite_boot_enabled = client
        .is_infinite_boot_enabled()
        .await
        .map_err(map_redfish_error)?;

    // If this is an nvswitch, don't set a boot order.
    let boot_order = match is_switch || is_powershelf {
        true => {
            tracing::debug!("Skipping boot order for nvswitch or powershelf");
            None
        }
        false => fetch_boot_order(client, &system)
            .await
            .inspect_err(|error| tracing::warn!(%error, "Failed to fetch boot order."))
            .ok(),
    };

    Ok(ComputerSystem {
        ethernet_interfaces,
        id: system.id,
        manufacturer: system.manufacturer,
        model: system.model,
        serial_number: system.serial_number,
        attributes: ComputerSystemAttributes {
            nic_mode: nic_mode.map(IntoModel::into_model),
            is_infinite_boot_enabled,
        },
        pcie_devices,
        base_mac,
        power_state: system.power_state.into_model(),
        sku: system.sku,
        boot_order,
    })
}

async fn fetch_ethernet_interfaces(
    client: &dyn Redfish,
    fetch_system_interfaces: bool,
    fetch_bluefield_oob: bool,
) -> Result<Vec<EthernetInterface>, RedfishError> {
    let eth_if_ids: Vec<String> = match match fetch_system_interfaces {
        false => client.get_manager_ethernet_interfaces().await,
        true => client.get_system_ethernet_interfaces().await,
    } {
        Ok(ids) => ids,
        Err(e) => {
            match e {
                RedfishError::HTTPErrorCode { status_code, .. } if status_code == NOT_FOUND => {
                    // missing oob for DPUs is handled below
                    Vec::new()
                }
                _ => return Err(e),
            }
        }
    };
    let mut eth_ifs: Vec<EthernetInterface> = Vec::new();
    let mut oob_found = false;

    for iface_id in eth_if_ids.iter() {
        let iface = match fetch_system_interfaces {
            false => client.get_manager_ethernet_interface(iface_id).await,
            true => client.get_system_ethernet_interface(iface_id).await,
        }?;

        oob_found |= iface_id.to_lowercase().contains("oob");

        let mac_address = if let Some(iface_mac_address) = iface.mac_address {
            match deserialize_input_mac_to_address(&iface_mac_address).map_err(|e| {
                RedfishError::GenericError {
                    error: format!("MAC address not valid: {iface_mac_address} (err: {e})"),
                }
            }) {
                Ok(mac) => Ok(Some(mac)),
                Err(e) => {
                    if iface
                        .interface_enabled
                        .is_some_and(|is_enabled| !is_enabled)
                    {
                        // disabled interfaces sometimes populate the MAC address with junk,
                        // ignore this error and create the interface with an empty mac address
                        // in the exploration report
                        tracing::debug!(
                            "could not parse MAC address for a disabled interface {iface_id} (link_status: {:#?}): {e}",
                            iface.link_status
                        );
                        Ok(None)
                    } else {
                        Err(e)
                    }
                }
            }
        } else {
            Ok(None)
        }?;

        let uefi_device_path = if let Some(uefi_device_path) = iface.uefi_device_path {
            let path_as_version_string = UefiDevicePath::from_str(&uefi_device_path)
                .map_err(|error| RedfishError::GenericError { error })?;
            Some(path_as_version_string)
        } else {
            None
        };

        let iface = EthernetInterface {
            description: iface.description,
            id: iface.id,
            interface_enabled: iface.interface_enabled,
            mac_address,
            link_status: iface.link_status.map(|s| s.to_string()),
            uefi_device_path,
        };

        eth_ifs.push(iface);
    }

    if !oob_found && fetch_bluefield_oob {
        // Temporary workaround untill get_system_ethernet_interface will return oob interface information
        // Usually the workaround for not even being able to enumerate the interfaces
        // would be used. But if a future Bluefield BMC revision returns interfaces
        // but still misses the OOB interface, we would use this path.
        if let Some(oob_iface) = get_oob_interface(client).await? {
            eth_ifs.push(oob_iface);
        } else {
            return Err(RedfishError::GenericError {
                error: "oob interface missing for dpu".to_string(),
            });
        }
    }

    Ok(eth_ifs)
}

async fn get_oob_interface(
    client: &dyn Redfish,
) -> Result<Option<EthernetInterface>, RedfishError> {
    // If chassis.contains(&"MGX_NVSwitch_0".to_string()),
    // nvlink switch does not have oob interface. And, if we try
    // querying boot options over redfish, we will get a 404 error.
    // So just return Ok(None) here.
    if is_switch(client).await? || is_powershelf(client).await? {
        return Ok(None);
    }

    // Temporary workaround until oob mac would be possible to get via Redfish
    let boot_options = client.get_boot_options().await?;
    let mac_pattern = Regex::new(r"MAC\((?<mac>[[:alnum:]]+)\,").unwrap();
    let mut boot_order_first_ethernet_interface = None;

    for option in boot_options.members.iter() {
        // odata_id: "/redfish/v1/Systems/Bluefield/BootOptions/Boot0001"
        let option_id = option.odata_id.split('/').next_back().unwrap();
        let boot_option = client.get_boot_option(option_id).await?;
        // display_name: "NET-OOB-IPV4"
        if boot_option.display_name.contains("OOB") {
            if boot_option.uefi_device_path.is_none() {
                // Try whether there might be other matching options
                continue;
            }
            // UefiDevicePath: "MAC(B83FD2909582,0x1)/IPv4(0.0.0.0,0x0,DHCP,0.0.0.0,0.0.0.0,0.0.0.0)/Uri()"
            if let Some(captures) =
                mac_pattern.captures(boot_option.uefi_device_path.unwrap().as_str())
            {
                let mac_addr_str = captures.name("mac").unwrap().as_str();
                let mut mac_addr_builder = String::new();

                // Transform B83FD2909582 -> B8:3F:D2:90:95:82
                for (i, c) in mac_addr_str.chars().enumerate() {
                    mac_addr_builder.push(c);
                    if ((i + 1) % 2 == 0) && ((i + 1) < mac_addr_str.len()) {
                        mac_addr_builder.push(':');
                    }
                }

                let mac_addr =
                    deserialize_input_mac_to_address(&mac_addr_builder).map_err(|e| {
                        RedfishError::GenericError {
                            error: format!("MAC address not valid: {mac_addr_builder} (err: {e})"),
                        }
                    })?;

                let (description, id) = if boot_option.display_name.contains("OOB") {
                    (
                        Some("1G DPU OOB network interface".to_string()),
                        Some("oob_net0".to_string()),
                    )
                } else {
                    (boot_option.description, Some(option_id.to_string()))
                };

                boot_order_first_ethernet_interface = Some(EthernetInterface {
                    description: description.clone(),
                    id: id.clone(),
                    interface_enabled: None,
                    mac_address: Some(mac_addr),
                    link_status: None,
                    uefi_device_path: None,
                });
            }
        }
    }

    Ok(boot_order_first_ethernet_interface)
}

async fn fetch_chassis(client: &dyn Redfish) -> Result<Vec<Chassis>, RedfishError> {
    let mut chassis: Vec<Chassis> = Vec::new();

    let chassis_list = client.get_chassis_all().await?;
    for chassis_id in &chassis_list {
        let Ok(desc) = client.get_chassis(chassis_id).await else {
            continue;
        };

        let net_adapter_list = if desc.network_adapters.is_some() {
            match client.get_chassis_network_adapters(chassis_id).await {
                Ok(v) => v,
                Err(RedfishError::NotSupported(_)) => vec![],
                // Nautobot uses Chassis_0 as the source of truth for the GB200 chassis serial number.
                // Other chassis subsystems with network adapters may report different serial numbers.
                Err(RedfishError::MissingKey { .. }) if chassis_id == "Chassis_0" => vec![],
                Err(_) => continue,
            }
        } else {
            vec![]
        };

        let mut net_adapters: Vec<NetworkAdapter> = Vec::new();
        for net_adapter_id in &net_adapter_list {
            let value = client
                .get_chassis_network_adapter(chassis_id, net_adapter_id)
                .await?;

            let net_adapter = NetworkAdapter {
                id: value.id,
                manufacturer: value.manufacturer,
                model: value.model,
                part_number: value.part_number,
                serial_number: Some(
                    value
                        .serial_number
                        .as_ref()
                        .unwrap_or(&"".to_string())
                        .trim()
                        .to_string(),
                ),
            };

            net_adapters.push(net_adapter);
        }

        // For GB200s, use the Chassis_0 assembly serial number to match Nautobot.
        let serial_number = if chassis_id == "Chassis_0" {
            client
                .get_chassis_assembly("Chassis_0")
                .await
                .ok()
                .and_then(|assembly| {
                    assembly
                        .assemblies
                        .iter()
                        .find(|asm| asm.model.as_deref() == Some("GB200 NVL"))
                        .and_then(|asm| asm.serial_number.clone())
                })
                .or(desc.serial_number)
        } else {
            desc.serial_number
        };

        let nvidia_oem = desc.oem.as_ref().and_then(|x| x.nvidia.as_ref());
        chassis.push(Chassis {
            id: chassis_id.to_string(),
            manufacturer: desc.manufacturer,
            model: desc.model,
            part_number: desc.part_number,
            serial_number,
            network_adapters: net_adapters,
            physical_slot_number: nvidia_oem.and_then(|x| x.chassis_physical_slot_number),
            compute_tray_index: nvidia_oem.and_then(|x| x.compute_tray_index),
            topology_id: nvidia_oem.and_then(|x| x.topology_id),
            revision_id: nvidia_oem.and_then(|x| x.revision_id),
        });
    }

    Ok(chassis)
}

async fn fetch_boot_order(
    client: &dyn Redfish,
    system: &libredfish::model::ComputerSystem,
) -> Result<BootOrder, RedfishError> {
    let boot_options_id =
        system
            .boot
            .boot_options
            .clone()
            .ok_or_else(|| RedfishError::MissingKey {
                key: "boot.boot_options".to_string(),
                url: system.odata.odata_id.to_string(),
            })?;

    let all_boot_options: Vec<libredfish::model::BootOption> = client
        .get_collection(boot_options_id)
        .await
        .and_then(|t1| t1.try_get::<libredfish::model::BootOption>())
        .into_iter()
        .flat_map(|x1| x1.members)
        .collect();

    let boot_order: Vec<BootOption> = system
        .boot
        .boot_order
        .iter()
        .filter_map(|ref_id| {
            all_boot_options
                .iter()
                .find(|opt| opt.boot_option_reference == *ref_id)
                .cloned()
                .map(IntoModel::into_model)
        })
        .collect();

    Ok(BootOrder { boot_order })
}

async fn fetch_pcie_devices(client: &dyn Redfish) -> Result<Vec<PCIeDevice>, RedfishError> {
    let pci_device_list = client.pcie_devices().await?;
    let mut pci_devices: Vec<PCIeDevice> = Vec::new();

    for pci_device in pci_device_list {
        pci_devices.push(PCIeDevice {
            description: pci_device.description,
            firmware_version: pci_device.firmware_version,
            id: pci_device.id.clone(),
            manufacturer: pci_device.manufacturer,
            gpu_vendor: pci_device.gpu_vendor,
            name: pci_device.name,
            part_number: pci_device.part_number,
            serial_number: pci_device.serial_number,
            status: pci_device.status.map(IntoModel::into_model),
        });
    }
    Ok(pci_devices)
}

async fn fetch_service(client: &dyn Redfish) -> Result<Vec<Service>, RedfishError> {
    let mut service: Vec<Service> = Vec::new();

    let inventory_list = client.get_software_inventories().await?;
    let mut inventories: Vec<Inventory> = Vec::new();
    for inventory_id in &inventory_list {
        let Ok(value) = client.get_firmware(inventory_id).await else {
            continue;
        };

        let inventory = Inventory {
            id: value.id,
            description: value.description,
            version: value.version,
            release_date: value.release_date,
        };

        inventories.push(inventory);
    }

    service.push(Service {
        id: "FirmwareInventory".to_string(),
        inventories,
    });

    Ok(service)
}

async fn fetch_machine_setup_status(
    client: &dyn Redfish,
    boot_interface_mac: Option<MacAddress>,
) -> Result<MachineSetupStatus, RedfishError> {
    let status = client
        .machine_setup_status(boot_interface_mac.map(BootInterfaceRef::Mac))
        .await?;
    let mut diffs: Vec<MachineSetupDiff> = Vec::new();

    for diff in status.diffs {
        diffs.push(MachineSetupDiff {
            key: diff.key,
            expected: diff.expected,
            actual: diff.actual,
        });
    }

    Ok(MachineSetupStatus {
        is_done: status.is_done,
        diffs,
    })
}

async fn fetch_secure_boot_status(client: &dyn Redfish) -> Result<SecureBootStatus, RedfishError> {
    let status = client.get_secure_boot().await?;

    let secure_boot_enable =
        status
            .secure_boot_enable
            .ok_or_else(|| RedfishError::GenericError {
                error: "expected secure_boot_enable_field set in secure boot response".to_string(),
            })?;

    let secure_boot_current_boot =
        status
            .secure_boot_current_boot
            .ok_or_else(|| RedfishError::GenericError {
                error: "expected secure_boot_current_boot set in secure boot response".to_string(),
            })?;

    let is_enabled = secure_boot_enable && secure_boot_current_boot.is_enabled();

    Ok(SecureBootStatus { is_enabled })
}

async fn fetch_lockdown_status(client: &dyn Redfish) -> Result<LockdownStatus, RedfishError> {
    let status = client.lockdown_status().await?;
    let internal_status = if status.is_fully_enabled() {
        InternalLockdownStatus::Enabled
    } else if status.is_fully_disabled() {
        InternalLockdownStatus::Disabled
    } else {
        InternalLockdownStatus::Partial
    };
    Ok(LockdownStatus {
        status: internal_status,
        message: status.message().to_string(),
    })
}

pub(crate) fn map_redfish_client_creation_error(
    error: RedfishClientCreationError,
) -> EndpointExplorationError {
    match error {
        RedfishClientCreationError::MissingCredentials { key } => {
            EndpointExplorationError::MissingCredentials {
                key,
                cause: "credentials are missing in the secret engine".into(),
            }
        }
        RedfishClientCreationError::SecretEngineError { cause } => {
            EndpointExplorationError::SecretsEngineError {
                cause: format!("secret engine error occurred: {cause:#}"),
            }
        }
        RedfishClientCreationError::RedfishError(e) => map_redfish_error(e),
        RedfishClientCreationError::InvalidHeader(original_error) => {
            EndpointExplorationError::Other {
                details: format!("RedfishClientError::InvalidHeader: {original_error}"),
            }
        }
        RedfishClientCreationError::MissingArgument(argument) => EndpointExplorationError::Other {
            details: format!("Missing argument to RedFish client: {argument}"),
        },
    }
}

pub(crate) fn map_redfish_error(error: RedfishError) -> EndpointExplorationError {
    match &error {
        RedfishError::NetworkError { url, source } => {
            let details = format!("url: {url};\nsource: {source};\nerror: {error}");
            if source.is_connect() {
                EndpointExplorationError::ConnectionRefused { details }
            } else if source.is_timeout() {
                EndpointExplorationError::ConnectionTimeout { details }
            } else {
                EndpointExplorationError::Unreachable {
                    details: Some(details),
                }
            }
        }
        RedfishError::HTTPErrorCode {
            status_code,
            response_body,
            url,
        } if *status_code == http::StatusCode::FORBIDDEN && url.contains("FirmwareInventory") => {
            EndpointExplorationError::VikingFWInventoryForbiddenError {
                details: format!(
                    "HTTP {status_code} at {url} - this is a known, intermittent issue for Vikings."
                ),
                response_body: Some(response_body.clone()),
                response_code: Some(status_code.as_u16()),
            }
        }
        RedfishError::HTTPErrorCode {
            status_code,
            response_body,
            url,
        } if *status_code == http::StatusCode::UNAUTHORIZED
            || *status_code == http::StatusCode::FORBIDDEN =>
        {
            let code_str = status_code.as_str();
            EndpointExplorationError::Unauthorized {
                details: format!("HTTP {status_code} {code_str} at {url}"),
                response_body: Some(response_body.clone()),
                response_code: Some(status_code.as_u16()),
            }
        }
        RedfishError::HTTPErrorCode {
            status_code,
            response_body,
            url,
        } => EndpointExplorationError::RedfishError {
            details: format!("HTTP {status_code} at {url}"),
            response_body: Some(response_body.clone()),
            response_code: Some(status_code.as_u16()),
        },
        RedfishError::JsonDeserializeError { url, body, source } => {
            EndpointExplorationError::RedfishError {
                details: format!("Failed to deserialize data from {url}: {source}"),
                response_body: Some(body.clone()),
                response_code: None,
            }
        }
        _ => EndpointExplorationError::RedfishError {
            details: error.to_string(),
            response_body: None,
            response_code: None,
        },
    }
}

/// Classify a platform-service [`PlatformRedfishError`] into the same
/// site-explorer error enum `map_redfish_error` produces for the legacy
/// client, so callers (notably `explore_endpoint`'s AvoidLockout and HPE
/// intermittent-401 ladder) keep keying off `Unauthorized` as before.
pub(crate) fn map_platform_error(error: PlatformRedfishError) -> EndpointExplorationError {
    match &error {
        PlatformRedfishError::Network { url, source } => EndpointExplorationError::Unreachable {
            details: Some(format!("url: {url};\nsource: {source};\nerror: {error}")),
        },
        PlatformRedfishError::HttpStatus {
            status_code,
            response_body,
            url,
        } if *status_code == http::StatusCode::FORBIDDEN.as_u16()
            && url.contains("FirmwareInventory") =>
        {
            EndpointExplorationError::VikingFWInventoryForbiddenError {
                details: format!(
                    "HTTP {status_code} at {url} - this is a known, intermittent issue for Vikings."
                ),
                response_body: Some(response_body.clone()),
                response_code: Some(*status_code),
            }
        }
        PlatformRedfishError::HttpStatus {
            status_code,
            response_body,
            url,
        } if error.is_unauthorized() => EndpointExplorationError::Unauthorized {
            details: format!("HTTP {status_code} at {url}"),
            response_body: Some(response_body.clone()),
            response_code: Some(*status_code),
        },
        PlatformRedfishError::HttpStatus {
            status_code,
            response_body,
            url,
        } => EndpointExplorationError::RedfishError {
            details: format!("HTTP {status_code} at {url}"),
            response_body: Some(response_body.clone()),
            response_code: Some(*status_code),
        },
        PlatformRedfishError::Deserialize { url, source } => {
            EndpointExplorationError::RedfishError {
                details: format!("Failed to deserialize data from {url}: {source}"),
                response_body: None,
                response_code: None,
            }
        }
        _ => EndpointExplorationError::RedfishError {
            details: error.to_string(),
            response_body: None,
            response_code: None,
        },
    }
}

/// Scrub a password from a platform-service error before it can reach logs or
/// the exploration report (mirrors the legacy `redact_password` for
/// `libredfish::RedfishError`).
fn redact_platform_password(
    error: PlatformRedfishError,
    password: &str,
) -> PlatformRedfishError {
    const REDACTED: &str = "REDACTED";
    let redact = |v: String| v.replace(password, REDACTED);
    match error {
        PlatformRedfishError::HttpStatus {
            url,
            status_code,
            response_body,
        } => PlatformRedfishError::HttpStatus {
            url,
            status_code,
            response_body: redact(response_body),
        },
        PlatformRedfishError::Generic(message) => PlatformRedfishError::Generic(redact(message)),
        err => err,
    }
}

fn nv_error_classifier(
    err: &carbide_redfish::nv_redfish::BmcError,
) -> Option<bmc_explorer::ErrorClass> {
    type BmcError = carbide_redfish::nv_redfish::BmcError;
    match err {
        BmcError::InvalidResponse { status, .. } => match *status {
            http::StatusCode::NOT_FOUND => Some(bmc_explorer::ErrorClass::NotFound),
            http::StatusCode::INTERNAL_SERVER_ERROR => {
                Some(bmc_explorer::ErrorClass::InternalServerError)
            }
            _ => None,
        },
        _ => None,
    }
}

fn nv_bmc_explore_config(
    boot_interface_mac: Option<MacAddress>,
) -> bmc_explorer::Config<'static, carbide_redfish::nv_redfish::RedfishBmc> {
    bmc_explorer::Config {
        boot_interface_mac,
        error_classifier: &nv_error_classifier,
        // Chosen arbitrarily: we want to wait a bit between tries,
        // but not for too long relative to the total exploration
        // time.
        retry_timeout: Duration::from_millis(1000),
    }
}

fn map_nv_redfish_explore_error(
    err: bmc_explorer::Error<carbide_redfish::nv_redfish::RedfishBmc>,
) -> EndpointExplorationError {
    type BmcError = carbide_redfish::nv_redfish::BmcError;
    use carbide_redfish::nv_redfish::Error;
    match err {
        bmc_explorer::Error::NvRedfish { context, err } => match err {
            Error::Bmc(err) => match err {
                BmcError::ReqwestError(err) => {
                    let details = format!(
                        "context: {context}; network error: {err}; source: {:?}",
                        err.source()
                    );
                    if err.is_connect() {
                        EndpointExplorationError::ConnectionRefused { details }
                    } else if err.is_timeout() {
                        EndpointExplorationError::ConnectionTimeout { details }
                    } else {
                        EndpointExplorationError::Unreachable {
                            details: Some(details),
                        }
                    }
                }
                BmcError::InvalidResponse { url, status, text } => {
                    match status {
                        // Disclaimer: this is original libredfish code...
                        http::StatusCode::FORBIDDEN
                            if url.to_string().contains("FirmwareInventory") =>
                        {
                            EndpointExplorationError::VikingFWInventoryForbiddenError {
                                details: format!(
                                    "HTTP {status} at {url} - this is a known, intermittent issue for Vikings."
                                ),
                                response_body: Some(text),
                                response_code: Some(status.as_u16()),
                            }
                        }
                        http::StatusCode::UNAUTHORIZED | http::StatusCode::FORBIDDEN => {
                            EndpointExplorationError::Unauthorized {
                                details: format!(
                                    "HTTP {status} {} at {context} ({url})",
                                    status.as_str()
                                ),
                                response_body: Some(text),
                                response_code: Some(status.as_u16()),
                            }
                        }
                        _ => EndpointExplorationError::RedfishError {
                            details: format!("HTTP {status} at {context} ({url})"),
                            response_body: Some(text),
                            response_code: Some(status.as_u16()),
                        },
                    }
                }
                BmcError::JsonError(err) => EndpointExplorationError::RedfishError {
                    details: format!("context: {context}; json error: {err}"),
                    response_body: None,
                    response_code: None,
                },
                err => EndpointExplorationError::RedfishError {
                    details: format!("context: {context}; error: {err}"),
                    response_body: None,
                    response_code: None,
                },
            },
            Error::Json(err) => EndpointExplorationError::RedfishError {
                details: format!("context: {context}; json error: {err}"),
                response_body: None,
                response_code: None,
            },
            err => EndpointExplorationError::RedfishError {
                details: format!("context: {context}; error: {err}"),
                response_body: None,
                response_code: None,
            },
        },
        err => EndpointExplorationError::Other {
            details: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{Arc, Mutex};

    use arc_swap::ArcSwap;
    use async_trait::async_trait;
    use carbide_redfish::libredfish::test_support::RedfishSim;
    use carbide_redfish::nv_redfish::NvRedfishClientPool;
    use carbide_redfish_platform_api::RedfishError as PlatformRedfishError;
    use carbide_redfish_platform_api::model::{
        BmcAccountPolicyRequest, BmcCredentials, BmcDeleteUserRequest, BmcPasswordRequest, BmcRef,
        BmcResetKind, BmcStatus, BmcUserRequest, BootOrderRequest, BootOrderStatus, BossController,
        ChassisResetRequest, CreateVolumeRequest, DecommissionRequest, DpuNicMode,
        DpuNicModeStatus, FirmwareInventory, FirmwareUpdateRequest, JobHandle, JobState,
        LockdownStatus, MachineSetupRequest, MachineSetupStatus, PowerAction, PowerState,
        SecureBootStatus, SelectedPlatform,
    };
    use carbide_redfish_platform_api::service::{
        BmcAccountOps, BmcResetOps, BootOrderOps, DpuOps, FirmwareOps, HostPowerOps, JobPollOps,
        LockdownOps, MachineSetupOps, PlatformSelection, RedfishPlatformService, SecureBootOps,
        StorageOps,
    };
    use carbide_secrets::credentials::Credentials;
    use libredfish::model::service_root::RedfishVendor;

    use super::RedfishClient;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 443)
    }

    /// A password-rotation call recorded by [`RecordingPlatform`], with the
    /// credentials the caller attached to the `BmcRef`.
    #[derive(Debug, PartialEq)]
    enum RecordedCall {
        ChangePassword {
            credentials: Option<BmcCredentials>,
            username: String,
            new_password: String,
        },
        SetAccountPolicy {
            credentials: Option<BmcCredentials>,
        },
    }

    /// Minimal `RedfishPlatformService` mock recording the account calls the
    /// rotation flow makes. All other capabilities are unreachable in these
    /// tests. (Local because `RedfishSim` does not implement the platform
    /// service trait yet.)
    #[derive(Default)]
    struct RecordingPlatform {
        calls: Mutex<Vec<RecordedCall>>,
    }

    #[async_trait]
    impl BmcAccountOps for RecordingPlatform {
        async fn ensure_user(
            &self,
            _bmc: BmcRef,
            _req: BmcUserRequest,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn delete_user(
            &self,
            _bmc: BmcRef,
            _req: BmcDeleteUserRequest,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn change_password(
            &self,
            bmc: BmcRef,
            req: BmcPasswordRequest,
        ) -> Result<(), PlatformRedfishError> {
            self.calls
                .lock()
                .unwrap()
                .push(RecordedCall::ChangePassword {
                    credentials: bmc.credentials,
                    username: req.username,
                    new_password: req.new_password,
                });
            Ok(())
        }

        async fn set_account_policy(
            &self,
            bmc: BmcRef,
            _req: BmcAccountPolicyRequest,
        ) -> Result<(), PlatformRedfishError> {
            self.calls
                .lock()
                .unwrap()
                .push(RecordedCall::SetAccountPolicy {
                    credentials: bmc.credentials,
                });
            Ok(())
        }
    }

    #[async_trait]
    impl PlatformSelection for RecordingPlatform {
        async fn selected_platform(
            &self,
            _bmc: BmcRef,
        ) -> Result<SelectedPlatform, PlatformRedfishError> {
            unimplemented!()
        }

        async fn probe_endpoint(&self, _address: SocketAddr) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl HostPowerOps for RecordingPlatform {
        async fn power_state(&self, _bmc: BmcRef) -> Result<PowerState, PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_power(
            &self,
            _bmc: BmcRef,
            _action: PowerAction,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl BmcResetOps for RecordingPlatform {
        async fn bmc_status(&self, _bmc: BmcRef) -> Result<BmcStatus, PlatformRedfishError> {
            unimplemented!()
        }

        async fn reset_bmc(
            &self,
            _bmc: BmcRef,
            _kind: BmcResetKind,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn reset_chassis(
            &self,
            _bmc: BmcRef,
            _req: ChassisResetRequest,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_bmc_time_utc(&self, _bmc: BmcRef) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl MachineSetupOps for RecordingPlatform {
        async fn apply_machine_setup(
            &self,
            _bmc: BmcRef,
            _req: MachineSetupRequest,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }

        async fn machine_setup_status(
            &self,
            _bmc: BmcRef,
        ) -> Result<MachineSetupStatus, PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_uefi_password(
            &self,
            _bmc: BmcRef,
            _password: String,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }

        async fn clear_nvram(&self, _bmc: BmcRef) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl BootOrderOps for RecordingPlatform {
        async fn set_dpu_first_boot(
            &self,
            _bmc: BmcRef,
            _req: BootOrderRequest,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }

        async fn boot_order_status(
            &self,
            _bmc: BmcRef,
        ) -> Result<BootOrderStatus, PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_infinite_boot(
            &self,
            _bmc: BmcRef,
            _enabled: bool,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl SecureBootOps for RecordingPlatform {
        async fn secure_boot_status(
            &self,
            _bmc: BmcRef,
        ) -> Result<SecureBootStatus, PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_secure_boot(
            &self,
            _bmc: BmcRef,
            _enabled: bool,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn add_certificate(
            &self,
            _bmc: BmcRef,
            _certificate: Vec<u8>,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl LockdownOps for RecordingPlatform {
        async fn lockdown_status(
            &self,
            _bmc: BmcRef,
        ) -> Result<LockdownStatus, PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_host_lockdown(
            &self,
            _bmc: BmcRef,
            _enabled: bool,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_bmc_lockdown(
            &self,
            _bmc: BmcRef,
            _enabled: bool,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl DpuOps for RecordingPlatform {
        async fn nic_mode(&self, _bmc: BmcRef) -> Result<DpuNicModeStatus, PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_nic_mode(
            &self,
            _bmc: BmcRef,
            _mode: DpuNicMode,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }

        async fn set_host_rshim(
            &self,
            _bmc: BmcRef,
            _enabled: bool,
        ) -> Result<(), PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl FirmwareOps for RecordingPlatform {
        async fn start_update(
            &self,
            _bmc: BmcRef,
            _req: FirmwareUpdateRequest,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }

        async fn firmware_inventory(
            &self,
            _bmc: BmcRef,
        ) -> Result<FirmwareInventory, PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl StorageOps for RecordingPlatform {
        async fn boss_controller(
            &self,
            _bmc: BmcRef,
        ) -> Result<Option<BossController>, PlatformRedfishError> {
            unimplemented!()
        }

        async fn decommission(
            &self,
            _bmc: BmcRef,
            _req: DecommissionRequest,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }

        async fn create_volume(
            &self,
            _bmc: BmcRef,
            _req: CreateVolumeRequest,
        ) -> Result<Option<JobHandle>, PlatformRedfishError> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl JobPollOps for RecordingPlatform {
        async fn poll(
            &self,
            _bmc: BmcRef,
            _job: &JobHandle,
        ) -> Result<JobState, PlatformRedfishError> {
            unimplemented!()
        }
    }

    impl RedfishPlatformService for RecordingPlatform {}

    fn build_redfish_client(platform: Arc<dyn RedfishPlatformService>) -> RedfishClient {
        let proxy_address = Arc::new(ArcSwap::new(Arc::new(None)));
        let nv_pool = Arc::new(NvRedfishClientPool::new(proxy_address));
        RedfishClient::new(platform, Arc::new(RedfishSim::default()), nv_pool)
    }

    /// Password rotation must first change the password on a `BmcRef` carrying
    /// the CURRENT credentials (factory-state BMCs only accept that login),
    /// and only then set the account policy on a fresh `BmcRef` carrying the
    /// NEW credentials.
    #[tokio::test]
    async fn set_bmc_root_password_changes_password_then_sets_policy_with_new_credentials() {
        let platform = Arc::new(RecordingPlatform::default());
        let redfish = build_redfish_client(platform.clone());

        let factory_creds = Credentials::UsernamePassword {
            username: "root".to_string(),
            password: "factory_pass".to_string(),
        };

        redfish
            .set_bmc_root_password(
                test_addr(),
                RedfishVendor::NvidiaDpu,
                factory_creds,
                "site_pass".to_string(),
            )
            .await
            .expect("password rotation should succeed");

        let calls = platform.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![
                RecordedCall::ChangePassword {
                    credentials: Some(BmcCredentials::new("root", "factory_pass")),
                    username: "root".to_string(),
                    new_password: "site_pass".to_string(),
                },
                RecordedCall::SetAccountPolicy {
                    credentials: Some(BmcCredentials::new("root", "site_pass")),
                },
            ]
        );
    }
}
