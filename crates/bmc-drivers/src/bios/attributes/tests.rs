use carbide_test_support::value_scenarios;
use serde_json::json;

use super::*;

fn settings(attributes: &[(&str, Value)]) -> BiosSettings {
    BiosSettings {
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect(),
    }
}

#[test]
fn desired_settings_cover_only_reported_attributes_and_keep_accepted_encodings() {
    let current = settings(&[
        ("SerialComm", json!("OnConRedir")),
        ("TpmSecurity", json!("Off")),
        ("IPv4HTTPSupport_009F", json!("Disabled")),
        ("IPv6HTTPSupport_00A0", json!("Enabled")),
    ]);
    let expected = [
        BiosAttribute::any_string("SerialComm", &["OnConRedirAuto", "OnConRedir"]),
        BiosAttribute::string("TpmSecurity", "On"),
        // Alternative spelling this BIOS does not expose.
        BiosAttribute::string("IntelProcVtd", "Enabled"),
        BiosAttribute::prefix_string("IPv4HTTPSupport", "Enabled"),
    ];

    assert_eq!(
        desired_settings(&expected, &current).expect("current values are accepted"),
        settings(&[
            ("SerialComm", json!("OnConRedir")),
            ("TpmSecurity", json!("On")),
            ("IPv4HTTPSupport_009F", json!("Enabled")),
        ])
    );
    assert_eq!(
        desired_settings(&expected, &settings(&[("SerialComm", json!("Off"))])),
        Err(IndeterminateAttribute {
            attribute: "SerialComm".to_string(),
        })
    );
}

#[test]
fn spelling_families_keep_the_bios_on_its_own_spelling() {
    const SECURITY_DEVICE: BiosAttribute = BiosAttribute::prefix_spelling(
        "SecurityDeviceSupport",
        &[("Enabled", &["Disabled"]), ("Enable", &["Disable"])],
    );
    value_scenarios!(run = |current: &str| desired_settings(
        &[SECURITY_DEVICE],
        &settings(&[("SecurityDeviceSupport_0123", json!(current))])
    )
    .map(|desired| desired.attributes["SecurityDeviceSupport_0123"].clone());
        "family value" {
            "Disabled" => Ok(json!("Enabled")),
            "Disable" => Ok(json!("Enable")),
            "Enable" => Ok(json!("Enable")),
        }
        "unknown spelling" {
            "Off" => Err(IndeterminateAttribute {
                attribute: "SecurityDeviceSupport_0123".to_string(),
            }),
        }
    );
}

#[test]
fn dell_serial_redirection_follows_the_port_address_format() {
    value_scenarios!(run = |address: &str| dell::idrac::serial_redirection(
        &settings(&[("SerialPortAddress", json!(address))])
    )
    .map(|attribute| attribute.value.to_string());
        "newer BIOS" {
            "Serial1Com1Serial2Com2" => ["OnConRedirAuto".to_string(), "Serial1Com2Serial2Com1".to_string()],
        }
        "older BIOS" {
            "Com2" => ["OnConRedir".to_string(), "Com1".to_string()],
        }
    );
}
