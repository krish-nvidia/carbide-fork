/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Serial-console capability drivers.

mod ami;
mod dell;
mod hpe;
mod lenovo;
mod nvidia;
mod supermicro;
mod support;

pub(crate) use ami::MegaRacConsole;
pub(crate) use dell::IdracConsole;
pub(crate) use hpe::IloConsole;
pub(crate) use lenovo::{Gb300Console, LenovoAmiConsole, XccConsole};
pub(crate) use nvidia::{BlueFieldConsole, VikingConsole};
pub(crate) use supermicro::SupermicroBmcConsole;
