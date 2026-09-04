/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::firmware::standard::StandardFirmware;

/// NVIDIA OpenBMC trays: standard multipart upload with `ForceUpdate`, since the
/// BMC otherwise skips images matching the installed version.
pub(crate) static OPENBMC_FIRMWARE: StandardFirmware = StandardFirmware {
    multipart_fallback: None,
    force_update: true,
};
