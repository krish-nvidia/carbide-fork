/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::firmware::standard::StandardFirmware;

/// NVIDIA DGX Viking: standard firmware operations; its AMI firmware uploads
/// through `upload` rather than `MultipartUpload`.
pub(crate) static VIKING_FIRMWARE: StandardFirmware = StandardFirmware {
    multipart_fallback: Some("/redfish/v1/UpdateService/upload"),
    force_update: false,
};
