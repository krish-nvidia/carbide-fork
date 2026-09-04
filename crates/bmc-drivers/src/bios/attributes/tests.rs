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
