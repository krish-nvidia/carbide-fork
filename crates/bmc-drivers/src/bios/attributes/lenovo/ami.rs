use super::super::BiosAttribute;

/// BIOS attributes NICo expects on AMI MegaRAC BIOS, including Lenovo HS350x-class systems.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("VMXEN", "Enable"),
    BiosAttribute::string("PCIS007", "Enabled"),
    BiosAttribute::integer("LEM0001", 3),
    BiosAttribute::string("NWSK000", "Enabled"),
    BiosAttribute::string("NWSK001", "Disabled"),
    BiosAttribute::string("NWSK006", "Enabled"),
    BiosAttribute::string("NWSK002", "Disabled"),
    BiosAttribute::string("NWSK007", "Disabled"),
    BiosAttribute::string("FBO001", "UEFI"),
    BiosAttribute::string("EndlessBoot", "Enabled"),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> =
    Some(BiosAttribute::string("EndlessBoot", "Enabled"));
