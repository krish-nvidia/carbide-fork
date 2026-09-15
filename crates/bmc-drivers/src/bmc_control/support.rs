/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Manager-control mechanics shared by several vendor drivers.

use bmc_platform::{DriverOutcome, OpCx, PlatformError};
use nv_redfish::Resource;
use nv_redfish::core::{Bmc, ODataId};
use serde_json::Value;

/// Posts an OEM action below the manager's `Actions/Oem/`.
pub(super) async fn manager_oem_action<B: Bmc>(
    cx: &OpCx<'_, B>,
    action: &str,
    payload: &Value,
) -> Result<DriverOutcome, PlatformError> {
    let target = ODataId::from(format!("{}/Actions/Oem/{action}", cx.manager()?.odata_id()));
    cx.post(&target, payload).await
}
