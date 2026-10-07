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

//! Capability operations on a connected BMC.
//!
//! `bmc.power()?.set(ResetType::On)` runs the power driver selection chose
//! for this BMC; callers never see drivers or operation contexts.

use std::sync::Arc;

use bmc_drivers::CatalogError;
use bmc_platform::{
    Accounts, Attestation, Bios, BiosSettings, BiosStatus, BmcControl, BootInterfaceSelector,
    BootOrder, BootOrderStatus, ClassifyBmcError, Console, ConsoleSpec, ConsoleStatus, Dpu,
    DpuStatus, DriverOutcome, EvidenceProgress, Firmware, FirmwareUpload, HostPrivilegeLevel,
    Lockdown, LockdownDesiredState, LockdownScope, LockdownStatus, ManagerSettings,
    ManagerSettingsStatus, NicMode, OperationReference, PlatformError, Power, RshimState,
    SecureBoot, SecureBootStatus, Storage,
};
use nv_redfish::Bmc;
use nv_redfish::account::ManagerAccountCreate;
use nv_redfish::resource::{PowerState, ResetType};
use nv_redfish::schema::certificate::Certificate;
use nv_redfish::schema::component_integrity::ComponentIntegrity;
use nv_redfish::schema::computer_system::BootUpdate;
use nv_redfish::schema::manager_account::ManagerAccount;
use nv_redfish::schema::secure_boot::SecureBootUpdate;
use nv_redfish::schema::software_inventory::SoftwareInventory;
use nv_redfish::schema::update_service::UpdateServiceSimpleUpdateAction;

use crate::ConnectedBmc;

/// The driver selected for one capability of a connected BMC.
///
/// Each operation builds a fresh operation context, so it observes the BMC's
/// current state rather than resources an earlier operation resolved.
pub struct BoundDriver<'a, D: ?Sized + 'a, B: Bmc + 'static> {
    driver: &'a D,
    bmc: &'a ConnectedBmc<B>,
}

impl<B: Bmc + 'static> ConnectedBmc<B>
where
    B::Error: ClassifyBmcError,
{
    /// The accounts driver selected for this BMC.
    pub fn accounts(&self) -> Result<BoundDriver<'_, dyn Accounts<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().accounts()?,
            bmc: self,
        })
    }

    /// The attestation driver selected for this BMC.
    pub fn attestation(&self) -> Result<BoundDriver<'_, dyn Attestation<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().attestation()?,
            bmc: self,
        })
    }

    /// The bios driver selected for this BMC.
    pub fn bios(&self) -> Result<BoundDriver<'_, dyn Bios<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().bios()?,
            bmc: self,
        })
    }

    /// The bmc control driver selected for this BMC.
    pub fn bmc_control(&self) -> Result<BoundDriver<'_, dyn BmcControl<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().bmc_control()?,
            bmc: self,
        })
    }

    /// The boot order driver selected for this BMC.
    pub fn boot_order(&self) -> Result<BoundDriver<'_, dyn BootOrder<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().boot_order()?,
            bmc: self,
        })
    }

    /// The console driver selected for this BMC.
    pub fn console(&self) -> Result<BoundDriver<'_, dyn Console<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().console()?,
            bmc: self,
        })
    }

    /// The dpu driver selected for this BMC.
    pub fn dpu(&self) -> Result<BoundDriver<'_, dyn Dpu<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().dpu()?,
            bmc: self,
        })
    }

    /// The firmware driver selected for this BMC.
    pub fn firmware(&self) -> Result<BoundDriver<'_, dyn Firmware<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().firmware()?,
            bmc: self,
        })
    }

    /// The lockdown driver selected for this BMC.
    pub fn lockdown(&self) -> Result<BoundDriver<'_, dyn Lockdown<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().lockdown()?,
            bmc: self,
        })
    }

    /// The power driver selected for this BMC.
    pub fn power(&self) -> Result<BoundDriver<'_, dyn Power<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().power()?,
            bmc: self,
        })
    }

    /// The secure boot driver selected for this BMC.
    pub fn secure_boot(&self) -> Result<BoundDriver<'_, dyn SecureBoot<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().secure_boot()?,
            bmc: self,
        })
    }

    /// The storage driver selected for this BMC.
    pub fn storage(&self) -> Result<BoundDriver<'_, dyn Storage<B>, B>, CatalogError> {
        Ok(BoundDriver {
            driver: self.drivers().storage()?,
            bmc: self,
        })
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Accounts<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Accounts::list`].
    pub async fn list(&self) -> Result<Vec<Arc<ManagerAccount>>, PlatformError> {
        self.driver.list(&self.bmc.operation_context()).await
    }

    /// See [`Accounts::create`].
    pub async fn create(
        &self,
        request: ManagerAccountCreate,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .create(&self.bmc.operation_context(), request)
            .await
    }

    /// See [`Accounts::delete`].
    pub async fn delete(&self, username: &str) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .delete(&self.bmc.operation_context(), username)
            .await
    }

    /// See [`Accounts::change_password`].
    pub async fn change_password(
        &self,
        account_username: &str,
        password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .change_password(&self.bmc.operation_context(), account_username, password)
            .await
    }

    /// See [`Accounts::change_username`].
    pub async fn change_username(
        &self,
        old_username: &str,
        new_username: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .change_username(&self.bmc.operation_context(), old_username, new_username)
            .await
    }

    /// See [`Accounts::apply_default_policy`].
    pub async fn apply_default_policy(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .apply_default_policy(&self.bmc.operation_context())
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Attestation<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Attestation::components`].
    pub async fn components(&self) -> Result<Vec<Arc<ComponentIntegrity>>, PlatformError> {
        self.driver.components(&self.bmc.operation_context()).await
    }

    /// See [`Attestation::firmware_for_component`].
    pub async fn firmware_for_component(
        &self,
        component_id: &str,
    ) -> Result<Arc<SoftwareInventory>, PlatformError> {
        self.driver
            .firmware_for_component(&self.bmc.operation_context(), component_id)
            .await
    }

    /// See [`Attestation::ca_certificate`].
    pub async fn ca_certificate(
        &self,
        component_id: &str,
    ) -> Result<Arc<Certificate>, PlatformError> {
        self.driver
            .ca_certificate(&self.bmc.operation_context(), component_id)
            .await
    }

    /// See [`Attestation::request_evidence`].
    pub async fn request_evidence(
        &self,
        component_id: &str,
        nonce: &[u8],
    ) -> Result<EvidenceProgress, PlatformError> {
        self.driver
            .request_evidence(&self.bmc.operation_context(), component_id, nonce)
            .await
    }

    /// See [`Attestation::poll_evidence`].
    pub async fn poll_evidence(
        &self,
        pending: &OperationReference,
    ) -> Result<EvidenceProgress, PlatformError> {
        self.driver
            .poll_evidence(&self.bmc.operation_context(), pending)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Bios<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Bios::apply`].
    pub async fn apply(
        &self,
        profile: &BiosSettings,
        boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .apply(&self.bmc.operation_context(), profile, boot_interface)
            .await
    }

    /// See [`Bios::status`].
    pub async fn status(
        &self,
        profile: &BiosSettings,
        boot_interface: Option<&BootInterfaceSelector>,
    ) -> Result<BiosStatus, PlatformError> {
        self.driver
            .status(&self.bmc.operation_context(), profile, boot_interface)
            .await
    }

    /// See [`Bios::reset`].
    pub async fn reset(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver.reset(&self.bmc.operation_context()).await
    }

    /// See [`Bios::clear_pending`].
    pub async fn clear_pending(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .clear_pending(&self.bmc.operation_context())
            .await
    }

    /// See [`Bios::change_uefi_password`].
    pub async fn change_uefi_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .change_uefi_password(
                &self.bmc.operation_context(),
                current_password,
                new_password,
            )
            .await
    }

    /// See [`Bios::clear_uefi_password`].
    pub async fn clear_uefi_password(
        &self,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .clear_uefi_password(&self.bmc.operation_context(), current_password)
            .await
    }

    /// See [`Bios::clear_tpm`].
    pub async fn clear_tpm(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver.clear_tpm(&self.bmc.operation_context()).await
    }

    /// See [`Bios::infinite_boot_enabled`].
    pub async fn infinite_boot_enabled(&self) -> Result<Option<bool>, PlatformError> {
        self.driver
            .infinite_boot_enabled(&self.bmc.operation_context())
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn BmcControl<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`BmcControl::reset`].
    pub async fn reset(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver.reset(&self.bmc.operation_context()).await
    }

    /// See [`BmcControl::reset_to_factory_defaults`].
    pub async fn reset_to_factory_defaults(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .reset_to_factory_defaults(&self.bmc.operation_context())
            .await
    }

    /// See [`BmcControl::set_ntp_servers`].
    pub async fn set_ntp_servers(
        &self,
        servers: &[String],
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_ntp_servers(&self.bmc.operation_context(), servers)
            .await
    }

    /// See [`BmcControl::set_utc_timezone`].
    pub async fn set_utc_timezone(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_utc_timezone(&self.bmc.operation_context())
            .await
    }

    /// See [`BmcControl::ipmi_over_lan_enabled`].
    pub async fn ipmi_over_lan_enabled(&self) -> Result<bool, PlatformError> {
        self.driver
            .ipmi_over_lan_enabled(&self.bmc.operation_context())
            .await
    }

    /// See [`BmcControl::set_ipmi_over_lan`].
    pub async fn set_ipmi_over_lan(&self, enabled: bool) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_ipmi_over_lan(&self.bmc.operation_context(), enabled)
            .await
    }

    /// See [`BmcControl::apply_settings`].
    pub async fn apply_settings(
        &self,
        profile: &ManagerSettings,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .apply_settings(&self.bmc.operation_context(), profile)
            .await
    }

    /// See [`BmcControl::settings_status`].
    pub async fn settings_status(&self) -> Result<ManagerSettingsStatus, PlatformError> {
        self.driver
            .settings_status(&self.bmc.operation_context())
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn BootOrder<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`BootOrder::status`].
    pub async fn status(
        &self,
        boot_interface_selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        self.driver
            .status(&self.bmc.operation_context(), boot_interface_selector)
            .await
    }

    /// See [`BootOrder::set_override`].
    pub async fn set_override(
        &self,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_override(&self.bmc.operation_context(), override_setting)
            .await
    }

    /// See [`BootOrder::configure`].
    pub async fn configure(
        &self,
        boot_interface_selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .configure(&self.bmc.operation_context(), boot_interface_selector)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Console<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Console::setup`].
    pub async fn setup(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver.setup(&self.bmc.operation_context()).await
    }

    /// See [`Console::status`].
    pub async fn status(&self) -> Result<ConsoleStatus, PlatformError> {
        self.driver.status(&self.bmc.operation_context()).await
    }

    /// See [`Console::spec`].
    pub async fn spec(&self) -> Result<ConsoleSpec, PlatformError> {
        self.driver.spec(&self.bmc.operation_context()).await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Dpu<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Dpu::status`].
    pub async fn status(&self) -> Result<DpuStatus, PlatformError> {
        self.driver.status(&self.bmc.operation_context()).await
    }

    /// See [`Dpu::set_nic_mode`].
    pub async fn set_nic_mode(&self, mode: NicMode) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_nic_mode(&self.bmc.operation_context(), mode)
            .await
    }

    /// See [`Dpu::set_host_rshim`].
    pub async fn set_host_rshim(&self, state: RshimState) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_host_rshim(&self.bmc.operation_context(), state)
            .await
    }

    /// See [`Dpu::enable_bmc_rshim`].
    pub async fn enable_bmc_rshim(&self) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .enable_bmc_rshim(&self.bmc.operation_context())
            .await
    }

    /// See [`Dpu::set_host_privilege_level`].
    pub async fn set_host_privilege_level(
        &self,
        level: HostPrivilegeLevel,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set_host_privilege_level(&self.bmc.operation_context(), level)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Firmware<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Firmware::inventory`].
    pub async fn inventory(&self) -> Result<Vec<Arc<SoftwareInventory>>, PlatformError> {
        self.driver.inventory(&self.bmc.operation_context()).await
    }

    /// See [`Firmware::multipart_update`].
    pub async fn multipart_update(
        &self,
        upload: FirmwareUpload,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .multipart_update(&self.bmc.operation_context(), upload)
            .await
    }

    /// See [`Firmware::simple_update`].
    pub async fn simple_update(
        &self,
        request: &UpdateServiceSimpleUpdateAction,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .simple_update(&self.bmc.operation_context(), request)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Lockdown<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Lockdown::status`].
    pub async fn status(&self) -> Result<LockdownStatus, PlatformError> {
        self.driver.status(&self.bmc.operation_context()).await
    }

    /// See [`Lockdown::set`].
    pub async fn set(
        &self,
        scope: LockdownScope,
        desired: LockdownDesiredState,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set(&self.bmc.operation_context(), scope, desired)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Power<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Power::state`].
    pub async fn state(&self) -> Result<Option<PowerState>, PlatformError> {
        self.driver.state(&self.bmc.operation_context()).await
    }

    /// See [`Power::ac_power_cycle_supported`].
    pub async fn ac_power_cycle_supported(&self) -> Result<bool, PlatformError> {
        self.driver
            .ac_power_cycle_supported(&self.bmc.operation_context())
            .await
    }

    /// See [`Power::set`].
    pub async fn set(&self, reset_type: ResetType) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .set(&self.bmc.operation_context(), reset_type)
            .await
    }

    /// See [`Power::chassis_reset`].
    pub async fn chassis_reset(
        &self,
        chassis_id: &str,
        reset_type: ResetType,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .chassis_reset(&self.bmc.operation_context(), chassis_id, reset_type)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn SecureBoot<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`SecureBoot::status`].
    pub async fn status(&self) -> Result<SecureBootStatus, PlatformError> {
        self.driver.status(&self.bmc.operation_context()).await
    }

    /// See [`SecureBoot::set`].
    pub async fn set(&self, update: &SecureBootUpdate) -> Result<DriverOutcome, PlatformError> {
        self.driver.set(&self.bmc.operation_context(), update).await
    }

    /// See [`SecureBoot::has_platform_key`].
    pub async fn has_platform_key(&self) -> Result<bool, PlatformError> {
        self.driver
            .has_platform_key(&self.bmc.operation_context())
            .await
    }

    /// See [`SecureBoot::add_platform_key`].
    pub async fn add_platform_key(&self, pem: &str) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .add_platform_key(&self.bmc.operation_context(), pem)
            .await
    }
}

impl<B: Bmc + 'static> BoundDriver<'_, dyn Storage<B>, B>
where
    B::Error: ClassifyBmcError,
{
    /// See [`Storage::boot_controller`].
    pub async fn boot_controller(&self) -> Result<Option<String>, PlatformError> {
        self.driver
            .boot_controller(&self.bmc.operation_context())
            .await
    }

    /// See [`Storage::decommission_controller`].
    pub async fn decommission_controller(
        &self,
        controller_id: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .decommission_controller(&self.bmc.operation_context(), controller_id)
            .await
    }

    /// See [`Storage::create_volume`].
    pub async fn create_volume(
        &self,
        controller_id: &str,
        volume_name: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        self.driver
            .create_volume(&self.bmc.operation_context(), controller_id, volume_name)
            .await
    }
}
