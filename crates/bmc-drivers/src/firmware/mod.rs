/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Firmware inventory and update capability drivers.

mod ami;
mod dell;
mod lenovo;
mod nvidia;
mod standard;
mod supermicro;
mod support;

pub(crate) use ami::MegaRacFirmware;
pub(crate) use dell::IdracFirmware;
pub(crate) use lenovo::XccFirmware;
pub(crate) use nvidia::{OpenBmcFirmware, VikingFirmware};
pub(crate) use standard::StandardFirmware;
pub(crate) use supermicro::SmcFirmware;
