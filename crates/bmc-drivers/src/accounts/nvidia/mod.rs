/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod bluefield;
mod openbmc;
mod switch;
mod viking;

pub(crate) use bluefield::BLUEFIELD_ACCOUNTS;
pub(crate) use openbmc::OPENBMC_ACCOUNTS;
pub(crate) use switch::SWITCH_ACCOUNTS;
pub(crate) use viking::VIKING_ACCOUNTS;
