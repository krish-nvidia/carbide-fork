/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::power::standard::{Restart, StandardPower};

/// DGX Viking cuts power to its DPUs on a Redfish restart, so the host
/// restarts over IPMI; the AMI firmware offers no AC power cycle.
pub(crate) static VIKING_POWER: StandardPower = StandardPower {
    restart: Restart::Ipmi,
    full_power_cycle: None,
};
