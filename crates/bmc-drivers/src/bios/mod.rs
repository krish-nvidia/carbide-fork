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

pub(crate) use ami::MegaRacBios;
pub(crate) use dell::IdracBios;
pub(crate) use lenovo::XccBios;
pub(crate) use nvidia::{BlueFieldBios, OpenBmcBios, VikingBios};
pub(crate) use standard::StandardBios;
