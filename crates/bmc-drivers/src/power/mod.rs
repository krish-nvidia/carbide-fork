/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Host power capability drivers.

mod dell;
mod delta;
mod hpe;
mod lenovo;
mod liteon;
mod nvidia;
mod standard;
mod supermicro;

pub(crate) use dell::IdracPower;
pub(crate) use delta::DeltaPowerShelfPower;
pub(crate) use hpe::IloPower;
pub(crate) use lenovo::{SR650_V4_POWER, Sr675V3OvxPower, XCC_POWER};
pub(crate) use liteon::LiteOnPowerShelfPower;
pub(crate) use nvidia::{OPENBMC_POWER, VIKING_POWER};
pub(crate) use standard::STANDARD_POWER;
pub(crate) use supermicro::SMC_POWER;
