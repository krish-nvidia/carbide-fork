/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod sr650_v4;
mod sr675_v3_ovx;
mod xcc;

pub(crate) use sr650_v4::Sr650V4Power;
pub(crate) use sr675_v3_ovx::Sr675V3OvxPower;
pub(crate) use xcc::XccPower;
