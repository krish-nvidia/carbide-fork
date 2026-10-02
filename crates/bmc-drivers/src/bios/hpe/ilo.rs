/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};

use crate::bios::standard::StandardBios;

/// HPE iLO BIOS behavior: staged settings are left in place rather than
/// reverted.
pub(crate) struct IloBios;

#[async_trait]
impl<B: Bmc> Bios<B> for IloBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn clear_pending(&self, _cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        Ok(DriverOutcome::complete())
    }
}
