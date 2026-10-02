/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! One-time boot override and persistent boot-order drivers.

mod ami;
mod dell;
mod hpe;
mod lenovo;
mod nvidia;
mod standard;
mod supermicro;

pub(crate) use ami::MegaRacBootOrder;
pub(crate) use dell::IdracBootOrder;
pub(crate) use hpe::IloBootOrder;
pub(crate) use lenovo::XccBootOrder;
pub(crate) use nvidia::{BlueFieldBootOrder, OpenBmcBootOrder, VikingBootOrder};
pub(crate) use standard::StandardBootOrder;
pub(crate) use supermicro::X13BootOrder;
