use super::super::BiosAttribute;

/// BIOS attributes NICo expects on NVIDIA BlueField DPU.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("HostPrivilegeLevel", "Restricted"),
    BiosAttribute::string("Host Privilege Level", "Restricted"),
    BiosAttribute::string("InternalCPUModel", "Embedded"),
    BiosAttribute::string("Internal CPU Model", "Embedded"),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> = None;
