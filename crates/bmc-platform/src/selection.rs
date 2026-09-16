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

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A BMC capability with its own driver trait and driver-map slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    Power,
    BmcControl,
    Bios,
    BootOrder,
    SecureBoot,
    Lockdown,
    Accounts,
    Firmware,
    Storage,
    Dpu,
    Attestation,
    Console,
}

impl Capability {
    /// Every capability in stable wire order; also the driver-map slot order.
    pub const ALL: [Self; 12] = [
        Self::Power,
        Self::BmcControl,
        Self::Bios,
        Self::BootOrder,
        Self::SecureBoot,
        Self::Lockdown,
        Self::Accounts,
        Self::Firmware,
        Self::Storage,
        Self::Dpu,
        Self::Attestation,
        Self::Console,
    ];

    /// The single source of each capability's wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::BmcControl => "bmc_control",
            Self::Bios => "bios",
            Self::BootOrder => "boot_order",
            Self::SecureBoot => "secure_boot",
            Self::Lockdown => "lockdown",
            Self::Accounts => "accounts",
            Self::Firmware => "firmware",
            Self::Storage => "storage",
            Self::Dpu => "dpu",
            Self::Attestation => "attestation",
            Self::Console => "console",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Capability {
    type Err = UnknownCapability;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == value)
            .ok_or_else(|| UnknownCapability(value.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("unknown capability {0:?}")]
pub struct UnknownCapability(String);

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_round_trip_through_their_wire_names() {
        for capability in Capability::ALL {
            let encoded = serde_json::to_string(&capability).expect("capability serializes");
            let decoded: Capability =
                serde_json::from_str(&encoded).expect("capability deserializes");
            assert_eq!(decoded, capability);
            assert_eq!(capability.as_str().parse::<Capability>(), Ok(capability));
        }
        assert!("nope".parse::<Capability>().is_err());
    }
}
