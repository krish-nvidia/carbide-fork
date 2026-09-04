/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

mod ami;
mod gb300;
mod xcc;

pub(crate) use ami::LENOVO_AMI_CONSOLE;
pub(crate) use gb300::GB300_CONSOLE;
pub(crate) use xcc::XCC_CONSOLE;
