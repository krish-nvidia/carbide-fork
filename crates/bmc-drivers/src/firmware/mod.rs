/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Firmware inventory and update capability drivers.

mod ami;
mod dell;
mod nvidia;
mod standard;
mod support;

pub(crate) use ami::MegaRacFirmware;
pub(crate) use dell::IdracFirmware;
pub(crate) use nvidia::{OpenBmcFirmware, VikingFirmware};
pub(crate) use standard::StandardFirmware;
