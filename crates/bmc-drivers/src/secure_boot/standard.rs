/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{DriverOutcome, OpCx, PlatformError, SecureBoot, SecureBootStatus};
use nv_redfish::computer_system::{SecureBoot as SecureBootResource, SecureBootCurrentBootType};
use nv_redfish::core::{Bmc, EntityTypeRef, ODataETag, ODataId, ReferenceLeaf};
use nv_redfish::schema::secure_boot::SecureBootUpdate;
use serde::Deserialize;
use serde_json::json;

const PLATFORM_KEY_DATABASE: &str = "PK";

/// Standard Redfish Secure Boot state and platform-key operations.
pub(crate) struct StandardSecureBoot;

#[async_trait]
impl<B: Bmc> SecureBoot<B> for StandardSecureBoot {
    async fn status(&self, cx: &OpCx<'_, B>) -> Result<SecureBootStatus, PlatformError> {
        status(cx).await
    }

    async fn set(
        &self,
        cx: &OpCx<'_, B>,
        update: &SecureBootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        set_state(cx, update).await
    }

    async fn has_platform_key(&self, cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
        has_platform_key(cx).await
    }

    async fn add_platform_key(
        &self,
        cx: &OpCx<'_, B>,
        pem: &str,
    ) -> Result<DriverOutcome, PlatformError> {
        add_platform_key(cx, pem).await
    }
}

async fn secure_boot_resource<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<SecureBootResource<B>, PlatformError> {
    cx.system()?
        .secure_boot()
        .await
        .map_err(|error| cx.map_redfish_error(error))?
        .ok_or(PlatformError::Unsupported)
}

async fn status<B: Bmc>(cx: &OpCx<'_, B>) -> Result<SecureBootStatus, PlatformError> {
    let secure_boot = secure_boot_resource(cx).await?;
    normalize_status(
        secure_boot.secure_boot_enable(),
        secure_boot.secure_boot_current_boot(),
    )
}

fn normalize_status(
    configured: Option<bool>,
    active: Option<SecureBootCurrentBootType>,
) -> Result<SecureBootStatus, PlatformError> {
    let Some(configured) = configured else {
        return Err(PlatformError::InvalidResponse {
            message: "SecureBootEnable is absent".to_string(),
        });
    };
    let active = match active {
        Some(SecureBootCurrentBootType::Enabled) => Some(true),
        Some(SecureBootCurrentBootType::Disabled) => Some(false),
        Some(SecureBootCurrentBootType::UnsupportedValue) => {
            return Err(PlatformError::InvalidResponse {
                message: "SecureBootCurrentBoot has an unsupported value".to_string(),
            });
        }
        None => None,
    };
    Ok(match active {
        Some(active) if active != configured => SecureBootStatus::Pending,
        _ if configured => SecureBootStatus::Enabled,
        _ => SecureBootStatus::Disabled,
    })
}

async fn set_state<B: Bmc>(
    cx: &OpCx<'_, B>,
    update: &SecureBootUpdate,
) -> Result<DriverOutcome, PlatformError> {
    let secure_boot = secure_boot_resource(cx).await?;
    cx.patch(secure_boot.raw().as_ref(), update).await
}

async fn has_platform_key<B: Bmc>(cx: &OpCx<'_, B>) -> Result<bool, PlatformError> {
    let secure_boot = secure_boot_resource(cx).await?;
    let collection = cx
        .bmc()
        .get::<CertificateCollection>(&certificate_collection_path(secure_boot.raw().odata_id()))
        .await
        .map_err(|error| cx.map_bmc_error(error))?;
    Ok(!collection.members.is_empty() || collection.member_count.unwrap_or_default() != 0)
}

async fn add_platform_key<B: Bmc>(
    cx: &OpCx<'_, B>,
    pem: &str,
) -> Result<DriverOutcome, PlatformError> {
    if pem.trim().is_empty() {
        return Err(PlatformError::InvalidResponse {
            message: "platform key PEM is empty".to_string(),
        });
    }
    let secure_boot = secure_boot_resource(cx).await?;
    cx.post(
        &certificate_collection_path(secure_boot.raw().odata_id()),
        &json!({"CertificateString": pem, "CertificateType": "PEM"}),
    )
    .await
}

fn certificate_collection_path(secure_boot: &ODataId) -> ODataId {
    format!(
        "{}/SecureBootDatabases/{PLATFORM_KEY_DATABASE}/Certificates",
        secure_boot.to_string().trim_end_matches('/')
    )
    .into()
}

#[derive(Deserialize)]
struct CertificateCollection {
    #[serde(rename = "@odata.id")]
    odata_id: ODataId,
    #[serde(rename = "@odata.etag", default)]
    etag: Option<ODataETag>,
    #[serde(rename = "Members", default)]
    members: Vec<ReferenceLeaf>,
    #[serde(rename = "Members@odata.count", default)]
    member_count: Option<u64>,
}

impl EntityTypeRef for CertificateCollection {
    fn odata_id(&self) -> &ODataId {
        &self.odata_id
    }

    fn etag(&self) -> Option<&ODataETag> {
        self.etag.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_state_that_differs_from_active_is_pending() {
        assert_eq!(
            normalize_status(Some(true), Some(SecureBootCurrentBootType::Disabled)).unwrap(),
            SecureBootStatus::Pending
        );
    }
}
