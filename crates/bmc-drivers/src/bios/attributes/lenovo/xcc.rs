use super::super::BiosAttribute;

/// BIOS attributes NICo expects on Lenovo ThinkSystem (XCC).
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("DevicesandIOPorts_COMPort1", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_ConsoleRedirection", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_SerialPortSharing", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_SPRedirection", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_COMPortActiveAfterBoot", "Enabled"),
    BiosAttribute::string("DevicesandIOPorts_SerialPortAccessMode", "Shared"),
    BiosAttribute::string("Processors_IntelVirtualizationTechnology", "Enabled"),
    BiosAttribute::string("Processors_SVMMode", "Enabled"),
    BiosAttribute::string("BootModes_SystemBootMode", "UEFIMode"),
    BiosAttribute::string("NetworkStackSettings_IPv4HTTPSupport", "Enabled"),
    BiosAttribute::string("NetworkStackSettings_IPv4PXESupport", "Disabled"),
    BiosAttribute::string("NetworkStackSettings_IPv6PXESupport", "Disabled"),
    BiosAttribute::string("BootModes_InfiniteBootRetry", "Enabled"),
    BiosAttribute::string("BootModes_PreventOSChangesToBootOrder", "Enabled"),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> = Some(BiosAttribute::string(
    "BootModes_InfiniteBootRetry",
    "Enabled",
));
