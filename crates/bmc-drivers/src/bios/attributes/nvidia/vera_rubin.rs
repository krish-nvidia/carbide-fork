use super::super::BiosAttribute;

/// BIOS attributes NICo expects on NVIDIA Vera Rubin NVL compute tray.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("TPM", "Enabled"),
    BiosAttribute::string("EmbeddedUefiShell", "Disabled"),
    BiosAttribute::bool("GpuExposeAsPcie", true),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> =
    Some(BiosAttribute::string("EmbeddedUefiShell", "Disabled"));
