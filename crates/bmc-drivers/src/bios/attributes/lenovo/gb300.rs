use super::super::BiosAttribute;

/// BIOS attributes NICo expects on Lenovo ThinkSystem GB300 NVL.
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("PCIS007", "PCIS007Enabled"),
    BiosAttribute::integer("LEM0001", 0),
    BiosAttribute::string("NWSK000", "NWSK000Enabled"),
    BiosAttribute::string("NWSK001", "NWSK001Disabled"),
    BiosAttribute::string("NWSK006", "NWSK006Enabled"),
    BiosAttribute::string("NWSK002", "NWSK002Disabled"),
    BiosAttribute::string("NWSK007", "NWSK007Disabled"),
    BiosAttribute::integer("LEM0003", 50),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> = Some(BiosAttribute::integer("LEM0003", 50));
