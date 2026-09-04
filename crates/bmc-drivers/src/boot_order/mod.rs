/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! One-time boot override and persistent boot-order drivers.

mod dell;
mod hpe;
mod nvidia;
mod standard;
mod supermicro;

pub(crate) use dell::IdracBootOrder;
pub(crate) use hpe::IloBootOrder;
pub(crate) use nvidia::OpenBmcBootOrder;
pub(crate) use standard::StandardBootOrder;
pub(crate) use supermicro::X13BootOrder;
