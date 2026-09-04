/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod openbmc;
mod viking;

pub(crate) use openbmc::OpenBmcLockdown;
pub(crate) use viking::VikingLockdown;
