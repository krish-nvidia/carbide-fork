/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Lenovo XCC power mechanics shared by the XCC power drivers.

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::oem::lenovo::SystemResetType;

/// Restores AC power through the XCC OEM system reset.
pub(super) async fn ac_power_cycle<B: Bmc>(cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>
where
    B::Error: ActionError,
{
    cx.system()
        .await?
        .oem_lenovo_actions()
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)?
        .system_reset(SystemResetType::AcPowerCycle)
        .await
        .map(DriverOutcome::from)
        .map_err(|error| cx.map_redfish_error(error))
}
