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

use serde::{Deserialize, Serialize};

/// Which `If-Match` value PATCH requests carry on this BMC.
///
/// Redfish requires `If-Match` on PATCH when the resource reports an ETag.
/// The transport already sends `*` when no ETag is known, so the only
/// deviation worth modelling is firmware that rejects its own ETag.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtagMode {
    /// Send the ETag the resource reported; the transport sends `*` when it reported none.
    #[default]
    Resource,
    /// Always send `*`: for firmware that requires `If-Match` but rejects its own ETag.
    Wildcard,
}
