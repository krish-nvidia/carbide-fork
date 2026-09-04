/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::firmware::standard::StandardFirmware;

/// Dell iDRAC: standard firmware operations; older iDRAC firmware does not
/// advertise its `MultipartUpload` endpoint.
pub(crate) static IDRAC_FIRMWARE: StandardFirmware = StandardFirmware {
    multipart_fallback: Some("/redfish/v1/UpdateService/MultipartUpload"),
    force_update: false,
};
