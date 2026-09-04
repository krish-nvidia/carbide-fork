use super::super::BiosAttribute;

/// BIOS attributes NICo expects on Dell PowerEdge (iDRAC).
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("InBandManageabilityInterface", "Disabled"),
    BiosAttribute::string("UefiVariableAccess", "Standard"),
    BiosAttribute::any_string("SerialComm", &["OnConRedirAuto", "OnConRedir"]),
    BiosAttribute::any_string("SerialPortAddress", &["Serial1Com2Serial2Com1", "Com1"]),
    BiosAttribute::string("FailSafeBaud", "115200"),
    BiosAttribute::string("ConTermType", "Vt100Vt220"),
    BiosAttribute::string("RedirAfterBoot", "Enabled"),
    BiosAttribute::string("SriovGlobalEnable", "Enabled"),
    BiosAttribute::string("TpmSecurity", "On"),
    BiosAttribute::string("Tpm2Hierarchy", "Enabled"),
    BiosAttribute::string("Tpm2Algorithm", "SHA256"),
    BiosAttribute::string("HttpDev1EnDis", "Enabled"),
    BiosAttribute::string("HttpDev1TlsMode", "None"),
    BiosAttribute::string("PxeDev1EnDis", "Disabled"),
];

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> =
    Some(BiosAttribute::string("BootSeqRetry", "Enabled"));
