/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod ami;
mod dell;
mod nvidia;
mod standard;

pub(crate) use ami::MEGARAC_FIRMWARE;
pub(crate) use dell::IDRAC_FIRMWARE;
pub(crate) use nvidia::{OPENBMC_FIRMWARE, VIKING_FIRMWARE};
pub(crate) use standard::STANDARD_FIRMWARE;
