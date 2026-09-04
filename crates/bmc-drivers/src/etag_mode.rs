/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! `If-Match` convention derived from a BMC's identity.

use bmc_platform::{EtagMode, PlatformIdentity};
use bmc_runtime::{IdentityField, IdentityMatcher, MatchPattern};

/// Returns how PATCH requests should carry `If-Match` on the BMC `identity` describes.
///
/// AMI MegaRAC firmware, including AMI-based Lenovo and NVIDIA DGX BMCs,
/// requires `If-Match` but rejects the ETag it served, so writes use the
/// wildcard. Everything else sends the resource ETag the DMTF way.
pub fn etag_mode(identity: &PlatformIdentity) -> EtagMode {
    let ami_firmware = [
        IdentityMatcher::new(
            IdentityField::ServiceRootVendor,
            MatchPattern::ContainsAsciiCaseInsensitive("ami".to_string()),
        ),
        IdentityMatcher::new(
            IdentityField::ServiceRootOemKey,
            MatchPattern::ExactAsciiCaseInsensitive("Ami".to_string()),
        ),
    ]
    .iter()
    .any(|matcher| matcher.matches(identity));
    if ami_firmware {
        EtagMode::Wildcard
    } else {
        EtagMode::Resource
    }
}
