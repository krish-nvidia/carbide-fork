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

//! `RedfishPlatformRuntime`: resolves credentials, builds a per-call session,
//! gathers identity, selects a plugin, and dispatches into capability impls.

use std::sync::Arc;

use async_trait::async_trait;

use carbide_redfish_platform_api::error::RedfishError;
use carbide_redfish_platform_api::model::{
    BmcAccountPolicyRequest, BmcDeleteUserRequest, BmcPasswordRequest, BmcRef, BmcResetKind,
    BmcStatus, BmcUserRequest, BootOrderRequest, BootOrderStatus, BossController,
    ChassisResetRequest, CreateVolumeRequest, DecommissionRequest, DpuNicMode, DpuNicModeStatus,
    FirmwareInventory, FirmwareUpdateRequest, JobHandle, JobState, LockdownStatus,
    MachineSetupRequest, MachineSetupStatus, MatchSpecificity, PlatformIdentity, PowerAction,
    PowerState, ResetTransport, SecureBootStatus, SelectedPlatform,
};
use carbide_redfish_platform_api::ops::PlatformExecutionContext;
use carbide_redfish_platform_api::plugin::PlatformPlugin;
use carbide_redfish_platform_api::service::{
    BmcAccountOps, BmcResetOps, BootOrderOps, DpuOps, FirmwareOps, HostPowerOps, JobPollOps,
    LockdownOps, MachineSetupOps, PlatformSelection, RedfishPlatformService, SecureBootOps,
    StorageOps,
};

use crate::ops_impl::NvRedfishOps;
use crate::registry::PlatformRegistry;
use crate::{AuthMode, BmcCredentialProvider};

/// The concrete platform service.
pub struct RedfishPlatformRuntime {
    registry: PlatformRegistry,
    credentials: Arc<dyn BmcCredentialProvider>,
    auth_mode: AuthMode,
}

/// The resolved state for a single operation.
struct Prepared {
    ops: NvRedfishOps,
    identity: PlatformIdentity,
    plugin: Arc<dyn PlatformPlugin>,
    specificity: MatchSpecificity,
}

impl Prepared {
    fn ctx(&self) -> PlatformExecutionContext<'_> {
        PlatformExecutionContext::new(&self.ops, &self.identity)
    }
}

impl RedfishPlatformRuntime {
    /// Construct a runtime from a registry, credential provider, and auth mode.
    pub fn new(
        registry: PlatformRegistry,
        credentials: Arc<dyn BmcCredentialProvider>,
        auth_mode: AuthMode,
    ) -> Self {
        Self {
            registry,
            credentials,
            auth_mode,
        }
    }

    /// Resolve credentials, connect, gather identity, and select the plugin --
    /// the full per-call preparation.
    async fn prepare(&self, bmc: &BmcRef) -> Result<Prepared, RedfishError> {
        // Callers that own credential selection (exploration ladder, password
        // rotation) attach credentials to the BmcRef; otherwise use the
        // provider (BMC MAC -> vault, as today).
        let creds = match &bmc.credentials {
            Some(creds) => creds.clone(),
            None => self.credentials.credentials_for(bmc).await?,
        };
        let ops =
            NvRedfishOps::connect(bmc.address, creds.username, creds.password, self.auth_mode)
                .await?;

        // Fast path: trust a caller-supplied plugin hint (typically the plugin
        // id stored for this endpoint during site exploration) when the plugin
        // is registered, skipping live identity-gathering. Identity stays the
        // cheap vendor/product already in hand; capabilities probe live, so they
        // never depend on a full identity here.
        if let Some(hint) = &bmc.platform_hint
            && let Some(plugin) = self.registry.find_by_id(hint)
        {
            let identity = ops.cheap_identity();
            let specificity = plugin.detect(&identity).unwrap_or(MatchSpecificity::Vendor);
            tracing::debug!(plugin = %hint, address = %bmc.address, "using platform plugin hint");
            return Ok(Prepared {
                ops,
                identity,
                plugin,
                specificity,
            });
        }

        // Cold path: gather full identity and select deterministically.
        let identity = ops.gather_identity().await?;
        let (plugin, specificity) = self.registry.select(&identity)?;
        tracing::debug!(
            plugin = %plugin.metadata().id,
            vendor = plugin.metadata().vendor,
            address = %bmc.address,
            "selected redfish platform plugin",
        );
        Ok(Prepared {
            ops,
            identity,
            plugin,
            specificity,
        })
    }
}

/// Build a [`SelectedPlatform`] from a prepared operation.
fn selected_platform(prepared: &Prepared) -> SelectedPlatform {
    let meta = prepared.plugin.metadata();
    let reset_transport = prepared
        .plugin
        .power()
        .map_or(ResetTransport::Redfish, |power| power.reset_transport());
    SelectedPlatform {
        plugin_id: meta.id.clone(),
        plugin_version: meta.plugin_version.to_string(),
        vendor: meta.vendor.to_string(),
        specificity: prepared.specificity,
        reset_transport,
        identity: prepared.identity.clone(),
    }
}

/// Helper: fetch a capability or return a structured `NotSupported`.
macro_rules! cap {
    ($prepared:expr, $accessor:ident, $what:literal) => {
        $prepared
            .plugin
            .$accessor()
            .ok_or_else(|| RedfishError::not_supported($what))?
    };
}

#[async_trait]
impl PlatformSelection for RedfishPlatformRuntime {
    async fn selected_platform(&self, bmc: BmcRef) -> Result<SelectedPlatform, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        Ok(selected_platform(&prepared))
    }

    async fn probe_endpoint(&self, address: std::net::SocketAddr) -> Result<(), RedfishError> {
        NvRedfishOps::probe(address).await
    }
}

#[async_trait]
impl HostPowerOps for RedfishPlatformRuntime {
    async fn power_state(&self, bmc: BmcRef) -> Result<PowerState, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, power, "host power")
            .power_state(&prepared.ctx())
            .await
    }

    async fn set_power(&self, bmc: BmcRef, action: PowerAction) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, power, "host power")
            .set_power(&prepared.ctx(), action)
            .await
    }
}

#[async_trait]
impl BmcResetOps for RedfishPlatformRuntime {
    async fn bmc_status(&self, bmc: BmcRef) -> Result<BmcStatus, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, bmc_reset, "bmc reset")
            .bmc_status(&prepared.ctx())
            .await
    }

    async fn reset_bmc(&self, bmc: BmcRef, kind: BmcResetKind) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, bmc_reset, "bmc reset")
            .reset_bmc(&prepared.ctx(), kind)
            .await
    }

    async fn reset_chassis(
        &self,
        bmc: BmcRef,
        req: ChassisResetRequest,
    ) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, bmc_reset, "bmc reset")
            .reset_chassis(&prepared.ctx(), req)
            .await
    }

    async fn set_bmc_time_utc(&self, bmc: BmcRef) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, bmc_reset, "bmc reset")
            .set_bmc_time_utc(&prepared.ctx())
            .await
    }
}

#[async_trait]
impl MachineSetupOps for RedfishPlatformRuntime {
    async fn apply_machine_setup(
        &self,
        bmc: BmcRef,
        req: MachineSetupRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, machine_setup, "machine setup")
            .apply_machine_setup(&prepared.ctx(), req)
            .await
    }

    async fn machine_setup_status(&self, bmc: BmcRef) -> Result<MachineSetupStatus, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, machine_setup, "machine setup")
            .machine_setup_status(&prepared.ctx())
            .await
    }

    async fn set_uefi_password(
        &self,
        bmc: BmcRef,
        password: String,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, machine_setup, "machine setup")
            .set_uefi_password(&prepared.ctx(), password)
            .await
    }

    async fn clear_nvram(&self, bmc: BmcRef) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, machine_setup, "machine setup")
            .clear_nvram(&prepared.ctx())
            .await
    }
}

#[async_trait]
impl BootOrderOps for RedfishPlatformRuntime {
    async fn set_dpu_first_boot(
        &self,
        bmc: BmcRef,
        req: BootOrderRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, boot_order, "boot order")
            .set_dpu_first_boot(&prepared.ctx(), req)
            .await
    }

    async fn boot_order_status(&self, bmc: BmcRef) -> Result<BootOrderStatus, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, boot_order, "boot order")
            .boot_order_status(&prepared.ctx())
            .await
    }

    async fn set_infinite_boot(
        &self,
        bmc: BmcRef,
        enabled: bool,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, boot_order, "boot order")
            .set_infinite_boot(&prepared.ctx(), enabled)
            .await
    }
}

#[async_trait]
impl SecureBootOps for RedfishPlatformRuntime {
    async fn secure_boot_status(&self, bmc: BmcRef) -> Result<SecureBootStatus, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, secure_boot, "secure boot")
            .secure_boot_status(&prepared.ctx())
            .await
    }

    async fn set_secure_boot(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, secure_boot, "secure boot")
            .set_secure_boot(&prepared.ctx(), enabled)
            .await
    }

    async fn add_certificate(
        &self,
        bmc: BmcRef,
        certificate: Vec<u8>,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, secure_boot, "secure boot")
            .add_certificate(&prepared.ctx(), certificate)
            .await
    }
}

#[async_trait]
impl LockdownOps for RedfishPlatformRuntime {
    async fn lockdown_status(&self, bmc: BmcRef) -> Result<LockdownStatus, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, lockdown, "lockdown")
            .lockdown_status(&prepared.ctx())
            .await
    }

    async fn set_host_lockdown(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, lockdown, "lockdown")
            .set_host_lockdown(&prepared.ctx(), enabled)
            .await
    }

    async fn set_bmc_lockdown(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, lockdown, "lockdown")
            .set_bmc_lockdown(&prepared.ctx(), enabled)
            .await
    }
}

#[async_trait]
impl BmcAccountOps for RedfishPlatformRuntime {
    async fn ensure_user(&self, bmc: BmcRef, req: BmcUserRequest) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, accounts, "bmc accounts")
            .ensure_user(&prepared.ctx(), req)
            .await
    }

    async fn delete_user(
        &self,
        bmc: BmcRef,
        req: BmcDeleteUserRequest,
    ) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, accounts, "bmc accounts")
            .delete_user(&prepared.ctx(), req)
            .await
    }

    async fn change_password(
        &self,
        bmc: BmcRef,
        req: BmcPasswordRequest,
    ) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, accounts, "bmc accounts")
            .change_password(&prepared.ctx(), req)
            .await
    }

    async fn set_account_policy(
        &self,
        bmc: BmcRef,
        req: BmcAccountPolicyRequest,
    ) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, accounts, "bmc accounts")
            .set_account_policy(&prepared.ctx(), req)
            .await
    }
}

#[async_trait]
impl DpuOps for RedfishPlatformRuntime {
    async fn nic_mode(&self, bmc: BmcRef) -> Result<DpuNicModeStatus, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, dpu, "dpu").nic_mode(&prepared.ctx()).await
    }

    async fn set_nic_mode(&self, bmc: BmcRef, mode: DpuNicMode) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, dpu, "dpu")
            .set_nic_mode(&prepared.ctx(), mode)
            .await
    }

    async fn set_host_rshim(&self, bmc: BmcRef, enabled: bool) -> Result<(), RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, dpu, "dpu")
            .set_host_rshim(&prepared.ctx(), enabled)
            .await
    }
}

#[async_trait]
impl FirmwareOps for RedfishPlatformRuntime {
    async fn start_update(
        &self,
        bmc: BmcRef,
        req: FirmwareUpdateRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, firmware, "firmware")
            .start_update(&prepared.ctx(), req)
            .await
    }

    async fn firmware_inventory(&self, bmc: BmcRef) -> Result<FirmwareInventory, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, firmware, "firmware")
            .firmware_inventory(&prepared.ctx())
            .await
    }
}

#[async_trait]
impl StorageOps for RedfishPlatformRuntime {
    async fn boss_controller(&self, bmc: BmcRef) -> Result<Option<BossController>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, storage, "storage")
            .boss_controller(&prepared.ctx())
            .await
    }

    async fn decommission(
        &self,
        bmc: BmcRef,
        req: DecommissionRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, storage, "storage")
            .decommission(&prepared.ctx(), req)
            .await
    }

    async fn create_volume(
        &self,
        bmc: BmcRef,
        req: CreateVolumeRequest,
    ) -> Result<Option<JobHandle>, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        cap!(prepared, storage, "storage")
            .create_volume(&prepared.ctx(), req)
            .await
    }
}

#[async_trait]
impl JobPollOps for RedfishPlatformRuntime {
    async fn poll(&self, bmc: BmcRef, job: &JobHandle) -> Result<JobState, RedfishError> {
        let prepared = self.prepare(&bmc).await?;
        match prepared.plugin.job_poll() {
            Some(poller) => poller.poll(&prepared.ctx(), job).await,
            None => Err(RedfishError::not_supported(
                "this plugin does not implement job polling",
            )),
        }
    }
}

impl RedfishPlatformService for RedfishPlatformRuntime {}
