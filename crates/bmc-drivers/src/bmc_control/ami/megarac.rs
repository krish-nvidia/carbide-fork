/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use nv_redfish::resource::ResetType;

use crate::bmc_control::standard::{FactoryDefaults, StandardBmcControl};

/// AMI MegaRAC accepts at most two NTP servers and only restarts through `ForceRestart`.
pub(crate) static MEGARAC_BMC_CONTROL: StandardBmcControl = StandardBmcControl {
    reset_type: ResetType::ForceRestart,
    max_ntp_servers: Some(2),
    factory_defaults: FactoryDefaults::Standard,
};
