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

//! Quirks: behavior a platform's driver keeps, but some models or firmware
//! releases deviate from.
//!
//! Selection rules attach quirks independently of the driver they select,
//! so quirks of overlapping rules combine, and drivers ask
//! [`crate::OpCx::has_quirk`] where the behavior differs.

use serde::{Deserialize, Serialize};

/// One model or firmware deviation a driver handles.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quirk {
    /// The BlueField BMC cannot read or switch the DPU mode through Redfish.
    BlueFieldNicModeUnreadable,
    /// The BlueField BMC answers a BIOS read on a DPU in NIC mode with a 500
    /// whose body still reports NIC mode.
    BlueFieldNicModeBiosError,
    /// The BlueField-3 system `Oem/Nvidia` resource times out on a DPU in
    /// NIC mode, so its actions are posted without reading it. BlueField-2
    /// never reads that resource.
    BlueFieldOemTimeoutInNicMode,
    /// The BlueField BMC spells `Host Privilege Level` and `Internal CPU
    /// Model` with spaces.
    BlueFieldSpacedBiosAttributeNames,
    /// The Supermicro MGX C2 host reaches the BMC over SSIF rather than KCS,
    /// so host IPMI access is the system `IPMIHostInterface` and there is no
    /// OEM `KCSInterface`.
    SupermicroMgxC2,
    /// The Supermicro BMC exposes the system `IPMIHostInterface`.
    SupermicroIpmiHostInterface,
}
