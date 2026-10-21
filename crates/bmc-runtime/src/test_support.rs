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

use std::collections::BTreeSet;

use bmc_drivers::{CapabilitySelection, DriverMap, ResolvedSelection, Rules};
use bmc_platform::{PlatformIdentity, Quirk};
use carbide_secrets::credentials::{BmcCredentialType, CredentialKey};
use mac_address::MacAddress;

use crate::BmcRef;

/// A BMC at `192.0.2.10:443` selecting standard drivers with `quirks`.
pub(crate) fn endpoint(quirks: BTreeSet<Quirk>) -> BmcRef {
    BmcRef::new(
        "192.0.2.10:443".parse().expect("valid socket address"),
        CredentialKey::BmcCredentials {
            credential_type: BmcCredentialType::BmcRoot {
                bmc_mac_address: MacAddress::new([2, 0, 0, 0, 0, 1]),
            },
        },
        PlatformIdentity::default(),
        ResolvedSelection {
            drivers: DriverMap::filled(CapabilitySelection::Standard),
            matched_rules: Vec::new(),
            quirks,
            hash: Rules::new(Vec::new())
                .expect("empty rules are valid")
                .hash(),
        },
    )
    .expect("root credential key is valid")
}
