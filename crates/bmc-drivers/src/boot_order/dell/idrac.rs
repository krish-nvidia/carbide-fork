/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use async_trait::async_trait;
use bmc_platform::{BootOrder, DriverOutcome, OpCx, PlatformError};
use nv_redfish::core::{ActionError, Bmc};
use nv_redfish::schema::computer_system::{BootSource, BootSourceOverrideEnabled, BootUpdate};
use serde_json::json;

use crate::boot_order::standard::StandardBootOrder;
use crate::dell;
use crate::resources::attribute_map;

/// Dell iDRAC boot behavior.
///
/// iDRAC rejects `BootSourceOverrideTarget` writes: a PXE or disk override is
/// the iDRAC `ServerBoot` first boot device, and a UEFI HTTP override is pinned
/// through the `HttpDev1*` BIOS attributes as a configuration job.
pub(crate) struct IdracBootOrder;

// Some iDRACs report `HttpDev1Uri` as read-only (MessageId `IDRAC.*.SYS410`)
// for reasons that have not been isolated; callers then fall back to DHCP.
fn read_only_attribute_is_unsupported(error: PlatformError) -> PlatformError {
    if dell::is_read_only_attribute(&error) {
        PlatformError::Unsupported
    } else {
        error
    }
}

#[async_trait]
impl<B: Bmc> BootOrder<B> for IdracBootOrder
where
    B::Error: ActionError,
{
    fn standard(&self) -> &dyn BootOrder<B> {
        &StandardBootOrder
    }

    async fn set_override(
        &self,
        cx: &OpCx<'_, B>,
        override_setting: &BootUpdate,
    ) -> Result<DriverOutcome, PlatformError> {
        let device = match override_setting.boot_source_override_target {
            Some(BootSource::UefiHttp) => None,
            Some(BootSource::Pxe) => Some("PXE"),
            Some(BootSource::Hdd) => Some("HDD"),
            _ => return Err(PlatformError::Unsupported),
        };
        if let Some(device) = device {
            let boot_once = match override_setting.boot_source_override_enabled {
                Some(BootSourceOverrideEnabled::Continuous) => "Disabled",
                Some(BootSourceOverrideEnabled::Disabled) => {
                    return Err(PlatformError::Unsupported);
                }
                _ => "Enabled",
            };
            return dell::patch_manager_attributes(
                cx,
                json!({
                    "ServerBoot.1.FirstBootDevice": device,
                    "ServerBoot.1.BootOnce": boot_once
                }),
                None,
            )
            .await;
        }
        let uri = override_setting
            .http_boot_uri
            .as_deref()
            .ok_or(PlatformError::Unsupported)?;
        dell::stage_bios_attributes(
            cx,
            &attribute_map([
                ("HttpDev1Uri", json!(uri)),
                ("HttpDev1EnDis", json!("Enabled")),
                ("HttpDev1DhcpEnDis", json!("Disabled")),
                ("HttpDev1Protocol", json!("IPv4")),
            ]),
        )
        .await
        .map_err(read_only_attribute_is_unsupported)
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
