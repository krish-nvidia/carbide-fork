/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BIOS capability drivers and declarative attribute data.

mod ami;
pub mod attributes;
mod dell;
mod lenovo;
mod nvidia;
mod standard;

pub(crate) use ami::MEGARAC_BIOS;
pub(crate) use dell::IdracBios;
pub(crate) use lenovo::XCC_BIOS;
pub(crate) use nvidia::{BLUEFIELD_BIOS, OPENBMC_BIOS, VIKING_BIOS};
pub(crate) use standard::STANDARD_BIOS;
