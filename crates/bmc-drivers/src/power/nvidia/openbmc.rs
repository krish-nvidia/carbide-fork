/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::json;

use crate::power::standard::{Anchor, OemPowerCycle, Restart, StandardPower};

/// NVIDIA OpenBMC platforms cycle auxiliary power through the BMC chassis.
pub(crate) static OPENBMC_POWER: StandardPower = StandardPower {
    restart: Restart::Redfish,
    full_power_cycle: Some(OemPowerCycle {
        anchor: Anchor::Chassis("BMC_0"),
        action: "NvidiaChassis.AuxPowerReset",
        payload: || json!({"ResetType": "AuxPowerCycle"}),
    }),
};
