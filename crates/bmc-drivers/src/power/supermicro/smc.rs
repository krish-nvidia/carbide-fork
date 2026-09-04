/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::json;

use crate::power::standard::{Anchor, OemPowerCycle, Restart, StandardPower};

/// Supermicro restores AC power through its OEM system reset.
pub(crate) static SMC_POWER: StandardPower = StandardPower {
    restart: Restart::Redfish,
    full_power_cycle: Some(OemPowerCycle {
        anchor: Anchor::System,
        action: "OemSystemExtensions.Reset",
        payload: || json!({"ResetType": "ACCycle"}),
    }),
};
