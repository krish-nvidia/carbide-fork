/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Declarative, platform-specific BIOS attribute data.
//!
//! Each family module exports the attributes NICo expects and, where the
//! platform has one, the attribute that enables infinite boot retries.

use std::fmt;

use bmc_platform::BiosSettings;
use serde_json::Value;
use thiserror::Error;

/// Dell platforms.
pub mod dell;
/// HPE platforms.
pub mod hpe;
/// Lenovo platforms.
pub mod lenovo;
/// NVIDIA platforms.
pub mod nvidia;
/// Supermicro platforms.
pub mod supermicro;

/// How a BIOS attribute name is compared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributeName {
    /// The exact attribute name.
    Exact(&'static str),
    /// A prefix of generated names such as Supermicro's `IPv4HTTPSupport_009F`.
    Prefix(&'static str),
}

impl AttributeName {
    /// Returns the declared name or prefix.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact(value) | Self::Prefix(value) => value,
        }
    }

    /// Reports whether a reported attribute name is covered by this declaration.
    pub fn matches(self, candidate: &str) -> bool {
        match self {
            Self::Exact(value) => candidate == value,
            Self::Prefix(value) => candidate.starts_with(value),
        }
    }
}

/// One or more values accepted for a BIOS attribute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributeValue {
    /// An exact string value.
    String(&'static str),
    /// Equivalent encodings, typically a current and a legacy spelling.
    AnyString(&'static [&'static str]),
    /// A boolean value.
    Bool(bool),
    /// An integer value.
    Integer(i64),
}

impl AttributeValue {
    /// Reports whether a reported value satisfies this expectation.
    pub fn matches(self, actual: &Value) -> bool {
        match self {
            Self::String(expected) => actual.as_str() == Some(expected),
            Self::AnyString(expected) => actual
                .as_str()
                .is_some_and(|actual| expected.contains(&actual)),
            Self::Bool(expected) => actual.as_bool() == Some(expected),
            Self::Integer(expected) => actual.as_i64() == Some(expected),
        }
    }

    fn desired(self, current: Option<&Value>) -> Option<Value> {
        if let Some(current) = current
            && self.matches(current)
        {
            return Some(current.clone());
        }
        match self {
            Self::String(value) => Some(Value::String(value.to_string())),
            Self::AnyString(_) => None,
            Self::Bool(value) => Some(Value::Bool(value)),
            Self::Integer(value) => Some(Value::from(value)),
        }
    }
}

impl fmt::Display for AttributeValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => formatter.write_str(value),
            Self::AnyString(values) => write!(formatter, "any({})", values.join(",")),
            Self::Bool(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
        }
    }
}

/// One declarative BIOS attribute expectation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BiosAttribute {
    /// How reported attribute names are matched.
    pub name: AttributeName,
    /// The value or values NICo expects.
    pub value: AttributeValue,
}

impl BiosAttribute {
    /// An exact attribute that must hold one string.
    pub const fn string(name: &'static str, value: &'static str) -> Self {
        Self {
            name: AttributeName::Exact(name),
            value: AttributeValue::String(value),
        }
    }

    /// An exact attribute that may hold any of several equivalent strings.
    pub const fn any_string(name: &'static str, values: &'static [&'static str]) -> Self {
        Self {
            name: AttributeName::Exact(name),
            value: AttributeValue::AnyString(values),
        }
    }

    /// An exact attribute that must hold a boolean.
    pub const fn bool(name: &'static str, value: bool) -> Self {
        Self {
            name: AttributeName::Exact(name),
            value: AttributeValue::Bool(value),
        }
    }

    /// An exact attribute that must hold an integer.
    pub const fn integer(name: &'static str, value: i64) -> Self {
        Self {
            name: AttributeName::Exact(name),
            value: AttributeValue::Integer(value),
        }
    }

    /// Every attribute with this name prefix must hold one string.
    pub const fn prefix_string(prefix: &'static str, value: &'static str) -> Self {
        Self {
            name: AttributeName::Prefix(prefix),
            value: AttributeValue::String(value),
        }
    }

    /// Every attribute with this name prefix may hold any of several strings.
    pub const fn prefix_any_string(prefix: &'static str, values: &'static [&'static str]) -> Self {
        Self {
            name: AttributeName::Prefix(prefix),
            value: AttributeValue::AnyString(values),
        }
    }

    /// Every attribute with this name prefix must hold a boolean.
    pub const fn prefix_bool(prefix: &'static str, value: bool) -> Self {
        Self {
            name: AttributeName::Prefix(prefix),
            value: AttributeValue::Bool(value),
        }
    }
}

/// Materializes declarative expectations into the exact settings the standard
/// BIOS driver compares and applies.
///
/// Only attributes the BIOS reports are materialized, so tables may list
/// alternative spellings (HPE's per-CPU-vendor virtualization keys, BlueField's
/// renamed attributes) without writing keys a given firmware lacks. When the
/// current value is one of several accepted encodings it is kept, so a BIOS is
/// never rewritten between equivalent legacy and current spellings.
pub fn desired_settings(
    expected: &[BiosAttribute],
    current: &BiosSettings,
) -> Result<BiosSettings, IndeterminateAttribute> {
    let mut attributes = BiosSettings::default();
    for expectation in expected {
        let names: Vec<&str> = current
            .attributes
            .keys()
            .filter(|name| expectation.name.matches(name))
            .map(String::as_str)
            .collect();
        for name in names {
            let desired = expectation
                .value
                .desired(current.attributes.get(name))
                .ok_or_else(|| IndeterminateAttribute {
                    attribute: name.to_string(),
                })?;
            attributes.attributes.insert(name.to_string(), desired);
        }
    }
    Ok(attributes)
}

/// An attribute accepts several values and the BIOS currently reports none of them.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("BIOS attribute {attribute} accepts several values and none is currently set")]
pub struct IndeterminateAttribute {
    /// The reported attribute name.
    pub attribute: String,
}

#[cfg(test)]
mod tests;
