/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use nv_redfish::resource::ResetType;
use serde_json::json;

use crate::bmc_control::standard::{FactoryDefaults, StandardBmcControl};

/// Supermicro restores manager defaults through the OEM `SmcManagerConfig.Reset` action.
pub(crate) static SMC_BMC_CONTROL: StandardBmcControl = StandardBmcControl {
    reset_type: ResetType::GracefulRestart,
    max_ntp_servers: None,
    factory_defaults: FactoryDefaults::Oem {
        action: "SmcManagerConfig.Reset",
        payload: || json!({"Option": "ClearConfig"}),
    },
};
