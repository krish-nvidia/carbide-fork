use super::super::BiosAttribute;

/// BIOS attributes NICo expects on NVIDIA DGX Viking.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::bool("AcpiSpcrConsoleRedirectionEnable", true),
    BiosAttribute::bool("ConsoleRedirectionEnable0", true),
    BiosAttribute::string("AcpiSpcrPort", "COM0"),
    BiosAttribute::string("AcpiSpcrFlowControl", "None"),
    BiosAttribute::string("AcpiSpcrBaudRate", "115200"),
    BiosAttribute::string("BaudRate0", "115200"),
    BiosAttribute::string("SriovSupport", "Enabled"),
    BiosAttribute::string("VTdSupport", "Enable"),
    BiosAttribute::string("Ipv4Http", "Enabled"),
    BiosAttribute::string("Ipv4Pxe", "Disabled"),
    BiosAttribute::string("Ipv6Http", "Enabled"),
    BiosAttribute::string("Ipv6Pxe", "Disabled"),
    BiosAttribute::string("NvidiaInfiniteboot", "Enable"),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> =
    Some(BiosAttribute::string("NvidiaInfiniteboot", "Enable"));
