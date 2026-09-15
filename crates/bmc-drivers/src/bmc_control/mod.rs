/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BMC manager-control capability drivers.

mod ami;
mod dell;
mod hpe;
mod standard;
mod supermicro;
mod support;

pub(crate) use ami::MegaRacBmcControl;
pub(crate) use dell::IdracBmcControl;
pub(crate) use hpe::IloBmcControl;
pub(crate) use standard::StandardBmcControl;
pub(crate) use supermicro::SmcBmcControl;
