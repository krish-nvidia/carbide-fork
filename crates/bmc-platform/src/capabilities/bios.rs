/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::collections::BTreeMap;

use async_trait::async_trait;
use nv_redfish::core::Bmc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DriverOutcome, OpCx, PlatformError};

/// BIOS attribute names and values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BiosSettings {
    /// BIOS attribute values by attribute name; nulls are omitted.
    pub attributes: BTreeMap<String, Value>,
}

/// One BIOS attribute whose reported value differs from the expectation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BiosDiff {
    /// Attribute name.
    pub key: String,
    /// Value NICo wants.
    pub expected: Value,
    /// Value the BMC reports; `None` when the BMC does not expose the attribute.
    pub actual: Option<Value>,
}

/// Whether expected BIOS settings are applied, with every difference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BiosStatus {
    /// True when `differences` is empty.
    pub is_applied: bool,
    /// Expected attributes whose pending value differs.
    pub differences: Vec<BiosDiff>,
}

/// Current, pending, and desired BIOS configuration operations.
#[async_trait]
pub trait Bios<B: Bmc>: Send + Sync {
    async fn current(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError>;

    async fn pending(&self, cx: &OpCx<'_, B>) -> Result<BiosSettings, PlatformError>;

    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<BiosStatus, PlatformError>;

    async fn apply(
        &self,
        cx: &OpCx<'_, B>,
        expected: &BiosSettings,
    ) -> Result<DriverOutcome, PlatformError>;

    async fn reset(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>;

    async fn clear_pending(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError>;

    /// Changes the UEFI administrator password; the driver knows the BIOS's password slot.
    async fn change_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
        new_password: &str,
    ) -> Result<DriverOutcome, PlatformError>;

    async fn clear_uefi_password(
        &self,
        cx: &OpCx<'_, B>,
        current_password: &str,
    ) -> Result<DriverOutcome, PlatformError>;
}
