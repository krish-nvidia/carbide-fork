/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod bluefield;
mod openbmc;
mod viking;

pub(crate) use bluefield::BLUEFIELD_BIOS;
pub(crate) use openbmc::OPENBMC_BIOS;
pub(crate) use viking::VIKING_BIOS;
