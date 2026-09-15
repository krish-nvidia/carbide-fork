/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod bluefield;
mod openbmc;
mod viking;

pub(crate) use bluefield::BlueFieldBios;
pub(crate) use openbmc::OpenBmcBios;
pub(crate) use viking::VikingBios;
