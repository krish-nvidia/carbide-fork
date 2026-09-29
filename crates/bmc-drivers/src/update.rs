/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Typed resource updates under the BMC's `If-Match` convention.

use std::future::Future;

use bmc_platform::{DriverOutcome, EtagMode, OpCx, PlatformError};
use nv_redfish::Error as RedfishError;
use nv_redfish::core::{Bmc, EntityTypeRef, ModificationResponse};
use serde::Serialize;

/// Writes `body` to `resource` through `typed`, the `nv-redfish` update that
/// sends the resource's ETag.
///
/// Typed updates always send the ETag the resource reported, which
/// [`EtagMode::Wildcard`] firmware rejects, so there the same body is sent as
/// a PATCH with `If-Match: *` instead.
pub(crate) async fn apply<B, R, U, T, F>(
    cx: &OpCx<'_, B>,
    resource: &R,
    body: &U,
    typed: F,
) -> Result<DriverOutcome, PlatformError>
where
    B: Bmc,
    R: EntityTypeRef,
    U: Serialize + Send + Sync,
    F: Future<Output = Result<ModificationResponse<T>, RedfishError<B>>>,
{
    match cx.etag_mode() {
        EtagMode::Resource => typed
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error)),
        EtagMode::Wildcard => {
            drop(typed);
            cx.patch(resource, body).await
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::header::IF_MATCH;
    use bmc_platform::BmcControl;
    use serde_json::json;

    use super::*;
    use crate::bmc_control::StandardBmcControl;
    use crate::test_support::{Fixture, body};

    #[tokio::test]
    async fn only_wildcard_bmcs_replace_the_resource_etag() {
        let bmc = Fixture::new("Contoso", "Server", "1", "BMC")
            .document(
                "/redfish/v1/Managers/BMC",
                json!({
                    "@odata.id": "/redfish/v1/Managers/BMC",
                    "Id": "BMC",
                    "Name": "Manager",
                    "NetworkProtocol": {"@odata.id": "/redfish/v1/Managers/BMC/NetworkProtocol"},
                }),
            )
            .document(
                "/redfish/v1/Managers/BMC/NetworkProtocol",
                json!({
                    "@odata.id": "/redfish/v1/Managers/BMC/NetworkProtocol",
                    "@odata.etag": "\"np-1\"",
                    "Id": "NetworkProtocol",
                    "Name": "Manager Network Protocol",
                }),
            )
            .build()
            .await;

        for (etag_mode, if_match) in [(EtagMode::Resource, "\"np-1\""), (EtagMode::Wildcard, "*")] {
            let cx = bmc.cx(etag_mode).await;
            StandardBmcControl
                .set_ipmi_over_lan(&cx, true)
                .await
                .expect("IPMI update succeeds");
            let writes = bmc.writes();
            assert_eq!(writes.len(), 1, "{etag_mode:?}");
            assert_eq!(writes[0].headers[IF_MATCH], if_match, "{etag_mode:?}");
            assert_eq!(
                body(&writes[0]),
                json!({"IPMI": {"ProtocolEnabled": true}}),
                "{etag_mode:?}"
            );
        }
    }
}
