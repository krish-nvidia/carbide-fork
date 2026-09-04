/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::json;

use crate::power::standard::{Anchor, OemPowerCycle, Restart, StandardPower};

/// XCC restores AC power through its OEM system reset.
pub(crate) const XCC_AC_POWER_CYCLE: OemPowerCycle = OemPowerCycle {
    anchor: Anchor::System,
    action: "LenovoComputerSystem.SystemReset",
    payload: || json!({"ResetType": "ACPowerCycle"}),
};

pub(crate) static XCC_POWER: StandardPower = StandardPower {
    restart: Restart::Redfish,
    full_power_cycle: Some(XCC_AC_POWER_CYCLE),
};

/// SR650 V4 cuts power to its DPUs on a Redfish restart, which breaks their
/// PXE boot, so the host restarts over IPMI instead.
/// <https://github.com/NVIDIA/bare-metal-manager-core/issues/347>
pub(crate) static SR650_V4_POWER: StandardPower = StandardPower {
    restart: Restart::Ipmi,
    full_power_cycle: Some(XCC_AC_POWER_CYCLE),
};
