/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Compiled BMC capability drivers, their selection rules, and write policy.

mod accounts;
mod attestation;
pub mod bios;
mod bmc_control;
mod boot_order;
mod console;
mod dell;
mod dpu;
mod drivers;
mod etag_mode;
mod firmware;
mod lockdown;
mod power;
mod resources;
mod rules;
mod secure_boot;
mod storage;

pub use drivers::drivers;
pub use etag_mode::etag_mode;
pub use rules::{built_in_rules, rules_with_overrides};
