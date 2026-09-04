/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod ami;
mod dell;
mod delta;
mod hpe;
mod lenovo;
mod liteon;
mod nvidia;
mod standard;

pub(crate) use ami::MEGARAC_ACCOUNTS;
pub(crate) use dell::IdracAccounts;
pub(crate) use delta::DELTA_POWER_SHELF_ACCOUNTS;
pub(crate) use hpe::ILO_ACCOUNTS;
pub(crate) use lenovo::XCC_ACCOUNTS;
pub(crate) use liteon::LITEON_POWER_SHELF_ACCOUNTS;
pub(crate) use nvidia::{BLUEFIELD_ACCOUNTS, OPENBMC_ACCOUNTS, SWITCH_ACCOUNTS, VIKING_ACCOUNTS};
pub(crate) use standard::STANDARD_ACCOUNTS;
