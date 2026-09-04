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

pub(crate) use ami::MEGARAC_CONSOLE;
pub(crate) use dell::IdracConsole;
pub(crate) use hpe::ILO_CONSOLE;
pub(crate) use lenovo::{GB300_CONSOLE, LENOVO_AMI_CONSOLE, XCC_CONSOLE};
pub(crate) use nvidia::{BLUEFIELD_CONSOLE, VIKING_CONSOLE};
pub(crate) use supermicro::SupermicroBmcConsole;
