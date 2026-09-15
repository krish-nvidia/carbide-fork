/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Firmware mechanics shared by several vendor drivers.

use bmc_platform::{OpCx, PlatformError};
use nv_redfish::core::Bmc;

use crate::firmware::standard::{advertised_multipart_uri, update_service};

/// The advertised `MultipartHttpPushUri`, or `fallback` for firmware that
/// implements multipart push without advertising it.
pub(super) async fn upload_uri<B: Bmc>(
    cx: &OpCx<'_, B>,
    fallback: &str,
) -> Result<String, PlatformError> {
    let service = update_service(cx).await?;
    Ok(advertised_multipart_uri(&service).unwrap_or_else(|| fallback.to_string()))
}
