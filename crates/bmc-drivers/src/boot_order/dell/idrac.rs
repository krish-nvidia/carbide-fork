/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{
    BootInterfaceSelector, BootOrder, BootOrderStatus, DriverOutcome, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;
use nv_redfish::schema::computer_system::{BootSource, BootUpdate};
use serde_json::json;

use crate::boot_order::standard::{configure, status};
use crate::dell;

/// Dell iDRAC boot behavior.
///
/// iDRAC rejects `BootSourceOverrideTarget` writes, so a UEFI HTTP override is
/// pinned through the `HttpDev1*` BIOS attributes as a configuration job.
pub(crate) struct IdracBootOrder;

async fn set_http_override<B: Bmc>(
    cx: &OpCx<'_, B>,
    override_setting: &BootUpdate,
) -> Result<DriverOutcome, PlatformError> {
    if override_setting.boot_source_override_target != Some(BootSource::UefiHttp) {
        return Err(PlatformError::Unsupported);
    }
    let uri = override_setting
        .http_boot_uri
        .as_deref()
        .ok_or(PlatformError::Unsupported)?;
    dell::stage_bios_attributes(
        cx,
        json!({
            "HttpDev1Uri": uri,
            "HttpDev1EnDis": "Enabled",
            "HttpDev1DhcpEnDis": "Disabled",
            "HttpDev1Protocol": "IPv4"
        }),
    )
    .await
    .map_err(read_only_attribute_is_unsupported)
}

// Some iDRACs report `HttpDev1Uri` as read-only (MessageId `IDRAC.*.SYS410`)
// for reasons that have not been isolated; callers then fall back to DHCP.
fn read_only_attribute_is_unsupported(error: PlatformError) -> PlatformError {
    match &error {
        PlatformError::Bmc {
            status: 400,
            message_id,
            message,
        } if message_id
            .as_deref()
            .is_some_and(|id| id.ends_with("SYS410"))
            || message.contains("SYS410") =>
        {
            PlatformError::Unsupported
        }
        _ => error,
    }
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for IdracBootOrder {
    async fn status(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<BootOrderStatus, PlatformError> {
        status(cx, selector).await
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        set_http_override(cx, override_setting).await
    }

    async fn configure(
        &self,
        cx: &OpCx<'_, B>,
        selector: &BootInterfaceSelector,
    ) -> Result<DriverOutcome, PlatformError> {
        configure(cx, selector).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_http_uri_rejection_is_unsupported() {
        let read_only = PlatformError::Bmc {
            status: 400,
            message_id: Some("IDRAC.2.9.SYS410".to_string()),
            message: "attribute is read-only".to_string(),
        };
        assert_eq!(
            read_only_attribute_is_unsupported(read_only),
            PlatformError::Unsupported
        );
        let other = PlatformError::Bmc {
            status: 400,
            message_id: Some("Base.1.0.GeneralError".to_string()),
            message: "bad request".to_string(),
        };
        assert_eq!(read_only_attribute_is_unsupported(other.clone()), other);
    }
}
