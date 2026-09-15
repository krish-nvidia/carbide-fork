/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::collections::BTreeMap;
use std::num::NonZeroU16;

use bmc_platform::{
    ConsoleSpec, ConsoleState, ConsoleStatus, DriverOutcome, EscapeSeq, OpCx, PlatformError,
};
use nv_redfish::core::Bmc;
use serde_json::{Map, Value};

use crate::resources::{self, patch_bios_attributes, selected_bios};

pub(super) const SSH_PORT: NonZeroU16 = NonZeroU16::new(22).expect("22 is nonzero");
pub(super) const DPU_SSH_PORT: NonZeroU16 = NonZeroU16::new(2200).expect("2200 is nonzero");
pub(super) const IPMI_PORT: NonZeroU16 = NonZeroU16::new(623).expect("623 is nonzero");

/// One BIOS attribute the console depends on.
///
/// `enabled[0]` is also the value `setup` writes, so the expectation table is
/// the single description of the console configuration. Values spelled
/// `true`/`false` are written as booleans, which is how those BIOSes report them.
#[derive(Clone, Copy)]
pub(super) struct AttrExpectation {
    pub(super) key: &'static str,
    pub(super) enabled: &'static [&'static str],
    pub(super) disabled: &'static [&'static str],
    /// Older firmware omits the attribute: it is then neither checked nor written.
    pub(super) optional: bool,
}

pub(super) const fn attr(
    key: &'static str,
    enabled: &'static [&'static str],
    disabled: &'static [&'static str],
) -> AttrExpectation {
    AttrExpectation {
        key,
        enabled,
        disabled,
        optional: false,
    }
}

pub(super) const fn optional_attr(
    key: &'static str,
    enabled: &'static [&'static str],
    disabled: &'static [&'static str],
) -> AttrExpectation {
    AttrExpectation {
        key,
        enabled,
        disabled,
        optional: true,
    }
}

fn attr_value(text: &str) -> Value {
    match text {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        other => Value::String(other.to_string()),
    }
}

/// Writes each attribute's first enabled value plus `write_only` values, so
/// the expectation table is the single description of the console setup.
pub(super) async fn setup_bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
    attrs: &[AttrExpectation],
    write_only: &[(&str, &str)],
) -> Result<DriverOutcome, PlatformError> {
    let current = bios_attributes(cx).await?;
    let attributes: Map<String, Value> = attrs
        .iter()
        .filter(|attr| !attr.optional || current.contains_key(attr.key))
        .map(|attr| (attr.key.to_string(), attr_value(attr.enabled[0])))
        .chain(
            write_only
                .iter()
                .map(|(key, value)| ((*key).to_string(), attr_value(value))),
        )
        .collect();
    patch_bios_attributes(cx, Value::Object(attributes)).await
}

pub(super) fn attr_status(
    attrs: &BTreeMap<String, Value>,
    expected: &[AttrExpectation],
) -> ConsoleStatus {
    let mut enabled = true;
    let mut disabled = true;
    let mut observed = 0;
    let mut message = Vec::new();
    for expectation in expected {
        let value = attrs.get(expectation.key);
        if value.is_none() && expectation.optional {
            continue;
        }
        observed += 1;
        let value = match value {
            Some(Value::String(value)) => value.clone(),
            Some(Value::Bool(value)) => value.to_string(),
            _ => "<missing>".to_string(),
        };
        message.push(format!("{}={value}", expectation.key));
        enabled &= expectation.enabled.contains(&value.as_str());
        disabled &=
            expectation.disabled.is_empty() || expectation.disabled.contains(&value.as_str());
    }
    let state = if observed == 0 {
        ConsoleState::Partial
    } else if enabled {
        ConsoleState::Enabled
    } else if disabled {
        ConsoleState::Disabled
    } else {
        ConsoleState::Partial
    };
    ConsoleStatus {
        state,
        message: message.join(", "),
    }
}

pub(super) async fn bios_attributes<B: Bmc>(
    cx: &OpCx<'_, B>,
) -> Result<BTreeMap<String, Value>, PlatformError> {
    Ok(resources::bios_attributes(&selected_bios(cx).await?.raw()))
}

pub(super) fn ipmi_sol_spec() -> Result<ConsoleSpec, PlatformError> {
    Ok(ConsoleSpec::IpmiSol {
        port: IPMI_PORT,
        escape_filter: EscapeSeq::pair(b'~', vec![b'.', b'B', b'?', 0x1a, 0x18])
            .map_err(spec_error)?,
    })
}

pub(super) fn spec_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::InvalidResponse {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn attrs(values: &[(&str, &str)]) -> BTreeMap<String, Value> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_string(), json!(value)))
            .collect()
    }

    #[test]
    fn aggregate_status_preserves_partial_and_missing_values() {
        let expected = [attr("a", &["on"], &["off"]), attr("b", &["on"], &["off"])];
        assert_eq!(
            attr_status(&attrs(&[("a", "on"), ("b", "on")]), &expected).state,
            ConsoleState::Enabled
        );
        assert_eq!(
            attr_status(&attrs(&[("a", "off"), ("b", "off")]), &expected).state,
            ConsoleState::Disabled
        );
        assert_eq!(
            attr_status(&attrs(&[("a", "on"), ("b", "off")]), &expected).state,
            ConsoleState::Partial
        );
        assert_eq!(
            attr_status(&attrs(&[("a", "on")]), &expected).state,
            ConsoleState::Partial
        );
    }

    #[test]
    fn boolean_spellings_are_written_as_booleans() {
        assert_eq!(attr_value("true"), Value::Bool(true));
        assert_eq!(attr_value("COM1"), Value::String("COM1".to_string()));
    }
}
