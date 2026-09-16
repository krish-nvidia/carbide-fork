/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Compiled BMC capability drivers and the platform rules that select them.

mod accounts;
mod attestation;
pub mod bios;
mod bmc_control;
mod boot_order;
mod console;
mod dell;
mod dpu;
mod drivers;
mod firmware;
mod lockdown;
mod power;
mod resources;
mod rules;
mod secure_boot;
mod selection;
mod storage;

pub use drivers::{Catalog, CatalogError, Driver, Drivers, PluginId, PluginIdError};
pub use rules::{built_in_rules, rules_with_overrides};
pub use selection::{
    CapabilitySelection, DriverMap, MatchedRule, ResolvedSelection, Rule, RuleError, Rules,
    SelectionError, SelectionHash,
};
