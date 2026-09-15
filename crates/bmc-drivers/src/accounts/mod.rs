/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! BMC local-account capability drivers.

mod dell;
mod delta;
mod hpe;
mod lenovo;
mod liteon;
mod nvidia;
mod standard;
mod support;

pub(crate) use dell::IdracAccounts;
pub(crate) use delta::DeltaPowerShelfAccounts;
pub(crate) use hpe::IloAccounts;
pub(crate) use lenovo::XccAccounts;
pub(crate) use liteon::LiteOnPowerShelfAccounts;
pub(crate) use nvidia::{BlueFieldAccounts, OpenBmcAccounts, SwitchAccounts, VikingAccounts};
pub(crate) use standard::StandardAccounts;
