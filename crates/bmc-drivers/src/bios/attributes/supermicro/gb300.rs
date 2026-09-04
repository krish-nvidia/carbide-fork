use super::super::BiosAttribute;

/// BIOS attributes NICo expects on Supermicro GB300 NVL compute tray.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("SecurityDeviceSupport", "Enabled"),
    BiosAttribute::bool("Socket0Pcie6DisableOptionROM", false),
    BiosAttribute::bool("Socket1Pcie6DisableOptionROM", false),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> = None;
