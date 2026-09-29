/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod bluefield;
mod openbmc;

pub(crate) use bluefield::BlueFieldBootOrder;
pub(crate) use openbmc::OpenBmcBootOrder;
