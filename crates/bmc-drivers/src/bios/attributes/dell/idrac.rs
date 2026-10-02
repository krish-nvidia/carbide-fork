use bmc_platform::BiosSettings;
use serde_json::Value;

use super::super::BiosAttribute;

/// BIOS attributes NICo expects on Dell PowerEdge (iDRAC).
pub const ATTRIBUTES: &[BiosAttribute] = &[
    BiosAttribute::string("InBandManageabilityInterface", "Disabled"),
    BiosAttribute::string("UefiVariableAccess", "Standard"),
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
    // Read-only and already `Uefi` on iDRAC 10, which leaves nothing to write.
    BiosAttribute::string("BootMode", "Uefi"),
];

/// Serial redirection in the format the BIOS uses: a `SerialPortAddress`
/// starting with `Serial1` marks the newer BIOS, which redirects
/// automatically on COM2.
pub fn serial_redirection(current: &BiosSettings) -> [BiosAttribute; 2] {
    let newer = current
        .attributes
        .get("SerialPortAddress")
        .and_then(Value::as_str)
        .is_some_and(|address| address.starts_with("Serial1"));
    if newer {
        [
            BiosAttribute::string("SerialComm", "OnConRedirAuto"),
            BiosAttribute::string("SerialPortAddress", "Serial1Com2Serial2Com1"),
        ]
    } else {
        [
            BiosAttribute::string("SerialComm", "OnConRedir"),
            BiosAttribute::string("SerialPortAddress", "Com1"),
        ]
    }
}

/// Attribute that enables infinite boot retries, when the platform has one.
pub const INFINITE_BOOT: Option<BiosAttribute> =
    Some(BiosAttribute::string("BootSeqRetry", "Enabled"));
