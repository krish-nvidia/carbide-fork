/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::attestation::standard::StandardComponentIntegrity;

/// NVIDIA HGX: standard ComponentIntegrity plus the GPU root-of-trust to
/// firmware-inventory mapping.
pub(crate) static HGX_ATTESTATION: StandardComponentIntegrity = StandardComponentIntegrity {
    firmware_inventory: Some(firmware_inventory_id),
};

/// Maps `HGX_IRoT_GPU_<n>` to `HGX_FW_GPU_<n>`; other components have no mapping.
fn firmware_inventory_id(component_id: &str) -> Option<String> {
    let gpu = component_id.strip_prefix("HGX_IRoT_GPU_")?;
    (!gpu.is_empty() && gpu.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("HGX_FW_GPU_{gpu}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_hgx_gpu_component_to_firmware_inventory() {
        assert_eq!(
            firmware_inventory_id("HGX_IRoT_GPU_12").as_deref(),
            Some("HGX_FW_GPU_12")
        );
        assert_eq!(firmware_inventory_id("HGX_IRoT_NVSwitch_0"), None);
        assert_eq!(firmware_inventory_id("prefix_HGX_IRoT_GPU_0"), None);
    }
}
