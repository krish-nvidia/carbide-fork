use super::super::BiosAttribute;

/// BIOS attributes NICo expects on HPE ProLiant (iLO).
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("IntelProcVtd", "Enabled"),
    BiosAttribute::string("ProcAmdIoVt", "Enabled"),
    BiosAttribute::string("ProcVirtualization", "Enabled"),
    BiosAttribute::string("Dhcpv4", "Enabled"),
    BiosAttribute::string("HttpSupport", "Auto"),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> = None;
