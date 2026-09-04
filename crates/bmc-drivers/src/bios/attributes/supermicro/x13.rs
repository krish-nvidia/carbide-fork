use super::super::BiosAttribute;

/// BIOS attributes NICo expects on Supermicro X13; names are prefixes because the BIOS appends a registry suffix.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::prefix_bool("QuietBoot", false),
    BiosAttribute::prefix_string("Re_tryBoot", "EFI Boot"),
    BiosAttribute::prefix_string("CSMSupport", "Disabled"),
    BiosAttribute::prefix_bool("SecureBootEnable", false),
    BiosAttribute::prefix_string("TXTSupport", "Enabled"),
    BiosAttribute::prefix_string("DeviceSelect", "TPM 2.0"),
    BiosAttribute::prefix_string("IntelVTforDirectedI_O_VT_d", "Enable"),
    BiosAttribute::prefix_string("IntelVirtualizationTechnology", "Enable"),
    BiosAttribute::prefix_string("SR-IOVSupport", "Enabled"),
    BiosAttribute::prefix_string("SR_IOVSupport", "Enabled"),
    BiosAttribute::prefix_string("IPv4HTTPSupport", "Enabled"),
    BiosAttribute::prefix_string("IPv4PXESupport", "Disabled"),
    BiosAttribute::prefix_string("IPv6HTTPSupport", "Disabled"),
    BiosAttribute::prefix_string("IPv6PXESupport", "Disabled"),
    BiosAttribute::prefix_any_string("SecurityDeviceSupport", &["Enabled", "Enable"]),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> = None;
