/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{Bios, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use serde_json::json;

use crate::bios::standard::StandardBios;
use crate::resources::{bios_attributes, bios_update, selected_bios};

/// Supermicro BIOS behavior: a TPM clear is a pending operation written to the
/// current BIOS resource, under a `PendingOperation*` name that varies by board.
pub(crate) struct SmcBios;

#[async_trait]
impl<B: Bmc> Bios<B> for SmcBios
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn Bios<B> {
        &StandardBios
    }

    async fn clear_tpm(&self, cx: &OpCx<'_, B>) -> Result<DriverOutcome, PlatformError> {
        let bios = selected_bios(cx).await?;
        let operation = bios_attributes(&bios.raw())
            .into_keys()
            .find(|key| key.starts_with("PendingOperation"))
            .ok_or(PlatformError::Unsupported)?;
        bios.update(&bios_update(json!({operation: "TPM Clear"}))?)
            .await
            .map(DriverOutcome::from)
            .map_err(|error| cx.map_redfish_error(error))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::*;
    use crate::test_support::{Fixture, body, path};

    const SYSTEM: &str = "/redfish/v1/Systems/1";
    const BIOS: &str = "/redfish/v1/Systems/1/Bios";

    #[tokio::test]
    async fn tpm_clear_writes_the_boards_pending_operation_to_the_current_bios() {
        for (attributes, expected) in [
            (
                json!({"PendingOperation_7A3C": "None", "QuietBoot": "Enabled"}),
                Some(json!({"Attributes": {"PendingOperation_7A3C": "TPM Clear"}})),
            ),
            (json!({"QuietBoot": "Enabled"}), None),
        ] {
            let bmc = Fixture::new("Supermicro", "", "1", "1")
                .document(
                    SYSTEM,
                    json!({"@odata.id": SYSTEM, "Id": "1", "Name": "System", "Bios": {"@odata.id": BIOS}}),
                )
                .document(
                    BIOS,
                    json!({"@odata.id": BIOS, "Id": "BIOS", "Name": "BIOS", "Attributes": attributes}),
                )
                .build()
                .await;
            let cx = bmc.cx().await;

            let result = SmcBios.clear_tpm(&cx).await;
            let writes = bmc.writes();
            match expected {
                Some(expected) => {
                    assert_eq!(result, Ok(DriverOutcome::complete()));
                    assert_eq!(writes.len(), 1);
                    assert_eq!(writes[0].method, Method::PATCH);
                    assert_eq!(path(&writes[0]), BIOS);
                    assert_eq!(body(&writes[0]), expected);
                }
                None => {
                    assert_eq!(result, Err(PlatformError::Unsupported));
                    assert!(writes.is_empty());
                }
            }
        }
    }
}
