/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! View-model builder for the admin-UI Configuration page.
//!
//! The page is generated rather than hand-written so it cannot drift from the
//! actual config surface. Three inputs are joined per option:
//!
//! - the configuration reference (`carbide_api_core::cfg::CONFIG_REFERENCE_MD`,
//!   i.e. `cfg/README.md`) supplies the catalog of documented options with
//!   their type, default, and description, organized into per-struct sections;
//! - the redacted effective `CarbideConfig`, serialized to JSON, supplies each
//!   option's current value (including values for options the reference does
//!   not document yet, which are appended as undocumented rows);
//! - `CarbideConfig::explicit_value_paths` supplies provenance: which dotted
//!   keys were explicitly set by a config file or `CARBIDE_API_*` environment
//!   variable, and by which source.

use std::collections::{BTreeMap, HashMap, HashSet};

/// Everything the Configuration page template needs.
pub(crate) struct ConfigPageView {
    pub groups: Vec<ConfigGroupView>,
}

pub(crate) struct ConfigGroupView {
    pub title: &'static str,
    /// Stable identifier used for the tab's `data-tab` / `id` attributes.
    pub slug: &'static str,
    pub sections: Vec<ConfigSectionView>,
}

pub(crate) struct ConfigSectionView {
    pub title: String,
    pub anchor: String,
    /// Dotted TOML path prefix of this section ("" for top-level options).
    pub path: String,
    pub rows: Vec<ConfigRowView>,
}

pub(crate) struct ConfigRowView {
    pub name: String,
    /// Full dotted path, also used by the client-side filter.
    pub path: String,
    pub ty: String,
    pub value_html: String,
    /// Render the value as a preformatted block instead of inline.
    pub multiline: bool,
    pub overridden: bool,
    /// Source label ("nico-api-config.toml", env, ...) when overridden.
    pub source: String,
    pub default_html: String,
    pub description_html: String,
    /// The option resolves to null/absent (unset optional value).
    pub unset: bool,
    pub undocumented: bool,
    /// The value shown is the live, runtime-adjustable value.
    pub runtime: bool,
}

/// Placeholder cells for absent content, kept to plain words -- no dashes
/// or empty strings, and one consistent word per situation.
const UNSET_HTML: &str = "<span class=\"config-unset\">unset</span>";
const EMPTY_HTML: &str = "<span class=\"config-unset\">empty</span>";
const NONE_HTML: &str = "<span class=\"config-unset\">none</span>";

/// A dynamic setting whose live value is folded into the catalog: attached to
/// the config option at `path` when one is documented, otherwise rendered as
/// its own row (with `description`) in that path's group.
pub(crate) struct RuntimeSetting {
    pub path: &'static str,
    /// Live value; `None` renders as "unset".
    pub value: Option<String>,
    /// Used only when no documented option matches `path`.
    pub description: &'static str,
}

/// One `| field | type | default | description |` row of the reference doc.
struct FieldDoc {
    name: String,
    ty: String,
    default: String,
    description: String,
}

/// Display groups, in page order. Sections land in a group via
/// `group_for_top_level_field`; unknown fields fall through to "Other".
const GROUP_ORDER: &[(&str, &str)] = &[
    ("Server & API", "server"),
    ("Networking", "networking"),
    ("Machines & Firmware", "machines"),
    ("Security", "security"),
    ("Hardware & Racks", "hardware"),
    ("Integrations & Observability", "integrations"),
    ("Other", "other"),
];

fn group_for_top_level_field(name: &str) -> &'static str {
    match name {
        "listen" | "listen_only" | "listen_mode" | "tls" | "database_url"
        | "max_database_connections" | "max_find_by_ids" | "auth" | "bypass_rbac"
        | "enable_admin_ui" | "web_ui_sidebar_tools" | "sitename" | "initial_objects_file" => {
            "Server & API"
        }
        "asn" | "dhcp_servers" | "ntp_servers" | "route_servers" | "enable_route_servers"
        | "deny_prefixes" | "site_fabric_prefixes" | "anycast_site_prefixes"
        | "common_tenant_host_asn" | "vpc_isolation_behavior" | "networks" | "pools"
        | "fnn" | "internet_l3_vni" | "datacenter_asn" | "site_global_vpc_vni"
        | "bgp_leaf_session_password" | "vpc_peering_policy" | "vpc_peering_policy_on_existing"
        | "network_security_group" | "dpa_config" | "network_segment_state_controller"
        | "vpc_prefix_state_controller" | "dpa_interface_state_controller"
        | "dpu_network_monitor_pinger_type" => "Networking",
        "host_naming_strategy" | "machine_state_controller" | "machine_validation_config"
        | "auto_machine_repair_plugin" | "host_health" | "initial_domain_name"
        | "min_dpu_functioning_links" | "bom_validation" | "compute_allocation_enforcement"
        | "dpu_config" | "dpu_ipmi_tool_impl" | "dpu_ipmi_reboot_attempts" | "nvue_enabled"
        | "dpf" | "initial_dpu_agent_upgrade_policy" | "x86_pxe_boot_url_override"
        | "arm_pxe_boot_url_override" | "pxe_public_base_url" | "set_http_boot_uri_for_vendors"
        | "retained_boot_interface_window" | "host_models" | "firmware_global"
        | "machine_updater" | "max_concurrent_machine_updates" | "machine_update_run_interval"
        | "mlxconfig_profiles" | "supernic_firmware_profiles" | "bios_profiles"
        | "selected_profile" => "Machines & Firmware",
        "attestation_enabled" | "tpm_required" | "machine_identity" | "spdm"
        | "spdm_state_controller" | "measured_boot_collector" | "bmc_session_lockout_threshold"
        | "secrets" | "kms" => "Security",
        "site_explorer" | "ib_config" | "ib_fabrics" | "nvlink_config"
        | "rack_management_enabled" | "rms" | "rack_profiles" | "rack_state_controller"
        | "power_shelf_state_controller" | "switch_state_controller" | "power_manager_options"
        | "ib_partition_state_controller" | "component_manager" => "Hardware & Racks",
        "metrics_endpoint" | "alt_metric_prefix" | "tracing" | "observability" | "log_history"
        | "log_filter" | "vmaas_config" | "dsx_exchange_event_bus" | "mqtt" => {
            "Integrations & Observability"
        }
        _ => "Other",
    }
}

/// Builds the page view from the reference doc, the redacted effective config
/// (as JSON), and the explicitly-set key paths with their source labels.
pub(crate) fn build_config_page(
    reference_md: &str,
    effective: &serde_json::Value,
    explicit_paths: &BTreeMap<String, String>,
    runtime_settings: Vec<RuntimeSetting>,
) -> ConfigPageView {
    let sections = parse_reference(reference_md);

    // Grouped sections, keyed by group title. Each group lazily gets a
    // leading section for the top-level scalar options assigned to it.
    let mut grouped: HashMap<&'static str, Vec<ConfigSectionView>> = HashMap::new();
    let mut top_level_rows: HashMap<&'static str, Vec<ConfigRowView>> = HashMap::new();
    let mut documented_top_level: HashSet<String> = HashSet::new();

    let top_level = sections
        .iter()
        .find(|(name, _)| name == "NicoConfig")
        .map(|(_, fields)| fields.as_slice())
        .unwrap_or(&[]);

    let section_by_anchor: HashMap<String, &Vec<FieldDoc>> = sections
        .iter()
        .map(|(name, fields)| (name.to_lowercase(), fields))
        .collect();

    for field in top_level {
        documented_top_level.insert(field.name.clone());
        let group = group_for_top_level_field(&field.name);
        match nested_section_for(field, &section_by_anchor) {
            Some(section_fields) => {
                let mut nested = Vec::new();
                build_section(
                    humanize(&field.name),
                    field.name.clone(),
                    section_fields,
                    &section_by_anchor,
                    effective,
                    explicit_paths,
                    &mut nested,
                    &mut HashSet::new(),
                );
                grouped.entry(group).or_default().extend(nested);
            }
            None => top_level_rows.entry(group).or_default().push(build_row(
                field,
                &field.name,
                effective,
                explicit_paths,
                false,
            )),
        }
    }

    // Options present in the effective config but missing from the reference
    // doc: surface them rather than silently dropping them.
    if let Some(object) = effective.as_object() {
        for (name, value) in object {
            if documented_top_level.contains(name) || documented_top_level.contains(&name.replace('-', "_")) {
                continue;
            }
            let field = FieldDoc {
                name: name.clone(),
                ty: String::new(),
                default: "—".to_string(),
                description: "Not yet documented in the configuration reference.".to_string(),
            };
            let mut row = build_row(&field, name, effective, explicit_paths, true);
            row.unset = value.is_null();
            top_level_rows
                .entry(group_for_top_level_field(name))
                .or_default()
                .push(row);
        }
    }

    // Fold live runtime values into their documented rows; settings without a
    // documented option become their own rows in the matching group.
    'settings: for setting in runtime_settings {
        let all_rows = top_level_rows
            .values_mut()
            .chain(grouped.values_mut().flatten().map(|s| &mut s.rows));
        for rows in all_rows {
            if let Some(row) = rows.iter_mut().find(|row| row.path == setting.path) {
                row.value_html = runtime_value_html(&setting);
                row.multiline = false;
                row.unset = setting.value.is_none();
                row.runtime = true;
                continue 'settings;
            }
        }
        let top = setting.path.split('.').next().unwrap_or(setting.path);
        top_level_rows
            .entry(group_for_top_level_field(top))
            .or_default()
            .push(ConfigRowView {
                name: setting.path.rsplit('.').next().unwrap_or(setting.path).to_string(),
                path: setting.path.to_string(),
                ty: String::new(),
                value_html: runtime_value_html(&setting),
                multiline: false,
                overridden: false,
                source: String::new(),
                default_html: NONE_HTML.to_string(),
                description_html: markdown_lite(setting.description),
                unset: setting.value.is_none(),
                undocumented: false,
                runtime: true,
            });
    }

    let mut groups = Vec::new();
    for (title, slug) in GROUP_ORDER {
        let mut sections = Vec::new();
        if let Some(rows) = top_level_rows.remove(title) {
            sections.push(ConfigSectionView {
                title: "Core Options".to_string(),
                anchor: format!("cfg-{slug}-core"),
                path: String::new(),
                rows,
            });
        }
        sections.extend(grouped.remove(title).unwrap_or_default());
        if !sections.is_empty() {
            groups.push(ConfigGroupView { title, slug, sections });
        }
    }

    ConfigPageView { groups }
}

/// If the field's type refers to a documented sub-struct section (and isn't a
/// collection of them), return that section's fields for recursion.
fn nested_section_for<'a>(
    field: &FieldDoc,
    sections: &'a HashMap<String, &Vec<FieldDoc>>,
) -> Option<&'a Vec<FieldDoc>> {
    let ty = clean_type(&field.ty);
    // Collections of structs stay leaf rows: their keys are operator-chosen,
    // so there is no fixed set of options to enumerate.
    if ty.contains("HashMap") || ty.contains("Vec<") || ty.contains("nested") {
        return None;
    }
    let inner = ty
        .trim_start_matches("Option<")
        .trim_end_matches('>')
        .trim();
    sections.get(&inner.to_lowercase()).copied()
}

/// Recursively emit a section for `fields` under `path`, appending nested
/// sub-struct sections after it. `visited` guards against reference cycles.
#[allow(clippy::too_many_arguments)]
fn build_section(
    title: String,
    path: String,
    fields: &[FieldDoc],
    sections: &HashMap<String, &Vec<FieldDoc>>,
    effective: &serde_json::Value,
    explicit_paths: &BTreeMap<String, String>,
    out: &mut Vec<ConfigSectionView>,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(path.clone()) {
        return;
    }
    let mut rows = Vec::new();
    let mut nested = Vec::new();
    for field in fields {
        let field_path = format!("{path}.{}", field.name);
        match nested_section_for(field, sections) {
            Some(section_fields) => build_section(
                format!("{title} · {}", humanize(&field.name)),
                field_path,
                section_fields,
                sections,
                effective,
                explicit_paths,
                &mut nested,
                visited,
            ),
            None => rows.push(build_row(field, &field_path, effective, explicit_paths, false)),
        }
    }
    out.push(ConfigSectionView {
        anchor: format!("cfg-{}", slugify(&path)),
        title,
        path,
        rows,
    });
    out.extend(nested);
}

fn build_row(
    field: &FieldDoc,
    path: &str,
    effective: &serde_json::Value,
    explicit_paths: &BTreeMap<String, String>,
    undocumented: bool,
) -> ConfigRowView {
    let value = lookup(effective, path);
    let (value_html, multiline) = match value {
        Some(value) => format_value(value),
        None => (UNSET_HTML.to_string(), false),
    };
    let unset = value.is_none_or(serde_json::Value::is_null);
    let source = explicit_source(path, explicit_paths);
    ConfigRowView {
        name: field.name.clone(),
        path: path.to_string(),
        ty: clean_type(&field.ty),
        value_html,
        multiline,
        overridden: source.is_some(),
        source: source.unwrap_or_default(),
        default_html: format_default(&field.default),
        description_html: match field.description.trim() {
            "" => "<span class=\"config-unset\">No description yet.</span>".to_string(),
            description => markdown_lite(description),
        },
        unset,
        undocumented,
        runtime: false,
    }
}

fn runtime_value_html(setting: &RuntimeSetting) -> String {
    match &setting.value {
        Some(value) => format!("<code>{}</code>", escape_html(value)),
        None => UNSET_HTML.to_string(),
    }
}

/// A key counts as explicitly set when the merged config sources provided it
/// or anything beneath it (e.g. `tls` is overridden when `tls.root_cafile_path`
/// was set in a file).
fn explicit_source(path: &str, explicit_paths: &BTreeMap<String, String>) -> Option<String> {
    // Figment normalizes TOML dashes; match both spellings.
    let underscored = path.replace('-', "_");
    let prefix = format!("{underscored}.");
    explicit_paths
        .iter()
        .find(|(key, _)| {
            let key = key.replace('-', "_");
            key == underscored || key.starts_with(&prefix)
        })
        .map(|(_, source)| source.clone())
}

fn lookup<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        let object = current.as_object()?;
        current = object
            .get(segment)
            .or_else(|| object.get(&segment.replace('_', "-")))?;
    }
    Some(current)
}

/// Renders a JSON value as HTML. Returns `(html, multiline)`; multiline values
/// are pretty-printed JSON meant for a `<pre>` block.
fn format_value(value: &serde_json::Value) -> (String, bool) {
    use serde_json::Value;
    match value {
        Value::Null => (UNSET_HTML.to_string(), false),
        Value::Bool(b) => (format!("<code>{b}</code>"), false),
        Value::Number(n) => (format!("<code>{n}</code>"), false),
        Value::String(s) if s.is_empty() => (EMPTY_HTML.to_string(), false),
        Value::String(s) => (format!("<code>{}</code>", escape_html(s)), false),
        Value::Array(items) if items.is_empty() => (EMPTY_HTML.to_string(), false),
        Value::Array(items) if items.iter().all(is_scalar) => {
            let joined = items.iter().map(scalar_text).collect::<Vec<_>>().join(", ");
            if joined.len() <= 120 {
                (format!("<code>{}</code>", escape_html(&joined)), false)
            } else {
                pretty(value)
            }
        }
        Value::Object(map) if map.is_empty() => (EMPTY_HTML.to_string(), false),
        // std::time::Duration serializes as {secs, nanos}; show it humanely.
        Value::Object(map)
            if map.len() == 2 && map.contains_key("secs") && map.contains_key("nanos") =>
        {
            let secs = map["secs"].as_u64().unwrap_or(0);
            let nanos = map["nanos"].as_u64().unwrap_or(0);
            if nanos == 0 {
                (format!("<code>{}</code>", escape_html(&humanize_seconds(secs))), false)
            } else {
                (format!("<code>{secs}.{nanos:09}s</code>"), false)
            }
        }
        _ => pretty(value),
    }
}

fn pretty(value: &serde_json::Value) -> (String, bool) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    (escape_html(&text), true)
}

fn is_scalar(value: &serde_json::Value) -> bool {
    !(value.is_array() || value.is_object())
}

fn scalar_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn humanize_seconds(secs: u64) -> String {
    if secs > 0 && secs.is_multiple_of(86400) {
        format!("{}d", secs / 86400)
    } else if secs > 0 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs > 0 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn format_default(default: &str) -> String {
    match default.trim() {
        "" | "—" | "-" | "*(see below)*" | "*(default)*" => NONE_HTML.to_string(),
        "**required**" => "<em>required</em>".to_string(),
        // An annotated absent default, e.g. "— (forever)".
        other => match other.strip_prefix("—") {
            Some(annotation) => format!("{NONE_HTML} {}", markdown_lite(annotation.trim())),
            None => markdown_lite(other),
        },
    }
}

/// Strips markdown emphasis from the reference's type cell for display.
fn clean_type(ty: &str) -> String {
    ty.trim().replace('`', "")
}

/// "dpa_config" -> "DPA", "machine_validation_config" -> "Machine Validation".
/// The `_config` suffix is dropped: every section is config, so it's noise.
fn humanize(field: &str) -> String {
    field
        .strip_suffix("_config")
        .unwrap_or(field)
        .split('_')
        .map(|word| match word {
            "dpu" => "DPU".to_string(),
            "dpa" => "DPA".to_string(),
            "dpf" => "DPF".to_string(),
            "ib" => "IB".to_string(),
            "nvlink" => "NVLink".to_string(),
            "rms" => "RMS".to_string(),
            "spdm" => "SPDM".to_string(),
            "tls" => "TLS".to_string(),
            "kms" => "KMS".to_string(),
            "fnn" => "FNN".to_string(),
            "vpc" => "VPC".to_string(),
            "mqtt" => "MQTT".to_string(),
            "oauth2" => "OAuth2".to_string(),
            "dsx" => "DSX".to_string(),
            "vmaas" => "VMaaS".to_string(),
            "bom" => "BOM".to_string(),
            "nsg" => "NSG".to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn slugify(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Parses the reference markdown into `(section_name, fields)` pairs in
/// document order. A section starts at a `## `/`### ` heading whose text is a
/// backticked struct name (e.g. ``### `TlsConfig` ``); prose headings without
/// tables contribute nothing.
fn parse_reference(markdown: &str) -> Vec<(String, Vec<FieldDoc>)> {
    let mut sections: Vec<(String, Vec<FieldDoc>)> = Vec::new();
    let mut current: Option<String> = None;
    let mut in_code_block = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("### ").or_else(|| trimmed.strip_prefix("## ")) {
            current = heading_struct_name(heading);
            if let Some(name) = &current
                && !sections.iter().any(|(existing, _)| existing == name)
            {
                sections.push((name.clone(), Vec::new()));
            }
            continue;
        }
        let Some(section) = &current else { continue };
        if let Some(field) = parse_table_row(trimmed)
            && let Some((_, fields)) = sections.iter_mut().find(|(name, _)| name == section)
        {
            fields.push(field);
        }
    }
    sections
}

/// Extracts the struct name from a heading like `` `TlsConfig` `` or
/// `` `NicoConfig` (top-level) ``; returns None for prose headings.
fn heading_struct_name(heading: &str) -> Option<String> {
    let rest = heading.trim().strip_prefix('`')?;
    let (name, _) = rest.split_once('`')?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Parses a markdown table row into a FieldDoc; header and separator rows
/// (and rows whose first cell isn't a backticked field name) return None.
fn parse_table_row(line: &str) -> Option<FieldDoc> {
    if !line.starts_with('|') {
        return None;
    }
    let cells: Vec<&str> = split_table_cells(line);
    if cells.len() < 4 {
        return None;
    }
    let name = cells[0].trim();
    let name = name.strip_prefix('`')?.strip_suffix('`')?;
    if name.is_empty() {
        return None;
    }
    Some(FieldDoc {
        name: name.to_string(),
        ty: cells[1].trim().to_string(),
        default: cells[2].trim().to_string(),
        description: cells[3..].join("|").trim().to_string(),
    })
}

/// Splits a markdown table row on unescaped pipes, honoring `\|` escapes.
fn split_table_cells(line: &str) -> Vec<&str> {
    let inner = line
        .trim()
        .trim_start_matches('|')
        .trim_end_matches('|');
    let mut cells = Vec::new();
    let mut start = 0;
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'|' && (i == 0 || bytes[i - 1] != b'\\') {
            cells.push(&inner[start..i]);
            start = i + 1;
        }
        i += 1;
    }
    cells.push(&inner[start..]);
    cells
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Minimal markdown renderer for the reference's table cells: HTML-escapes,
/// then supports `` `code` ``, `**bold**`, and `[text](target)` links (anchors
/// are rewritten to this page's section ids). Anything else stays plain text.
fn markdown_lite(text: &str) -> String {
    let escaped = escape_html(&text.replace("\\|", "|"));
    let linked = render_links(&escaped);
    let coded = render_delimited(&linked, "`", "<code>", "</code>");
    render_delimited(&coded, "**", "<strong>", "</strong>")
}

fn render_delimited(text: &str, delim: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start) = rest.find(delim) else {
            out.push_str(rest);
            return out;
        };
        let after = &rest[start + delim.len()..];
        let Some(end) = after.find(delim) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        out.push_str(open);
        out.push_str(&after[..end]);
        out.push_str(close);
        rest = &after[end + delim.len()..];
    }
}

fn render_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(open_bracket) = rest.find('[') else {
            out.push_str(rest);
            return out;
        };
        let Some(rel_close) = rest[open_bracket..].find("](") else {
            out.push_str(&rest[..open_bracket + 1]);
            rest = &rest[open_bracket + 1..];
            continue;
        };
        let close_bracket = open_bracket + rel_close;
        let target_start = close_bracket + 2;
        let Some(rel_end) = rest[target_start..].find(')') else {
            out.push_str(&rest[..open_bracket + 1]);
            rest = &rest[open_bracket + 1..];
            continue;
        };
        let target_end = target_start + rel_end;
        let label = &rest[open_bracket + 1..close_bracket];
        let target = &rest[target_start..target_end];
        out.push_str(&rest[..open_bracket]);
        if let Some(anchor) = target.strip_prefix('#') {
            // The reference's intra-doc anchors don't map 1:1 onto this
            // page's section ids, so keep the label but drop the link.
            let _ = anchor;
            out.push_str(label);
        } else if target.starts_with("http://") || target.starts_with("https://") {
            out.push_str(&format!("<a href=\"{target}\">{label}</a>"));
        } else {
            out.push_str(label);
        }
        rest = &rest[target_end + 1..];
    }
}

#[cfg(test)]
mod configuration_tests {
    use super::*;

    #[test]
    fn parses_real_reference() {
        let sections = parse_reference(carbide_api_core::cfg::CONFIG_REFERENCE_MD);
        let names: Vec<&str> = sections.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"NicoConfig"), "top-level section missing: {names:?}");
        assert!(names.contains(&"TlsConfig"));
        assert!(names.contains(&"SiteExplorerConfig"));

        let (_, top) = sections.iter().find(|(n, _)| n == "NicoConfig").unwrap();
        assert!(top.len() > 50, "expected many top-level fields, got {}", top.len());
        let listen = top.iter().find(|f| f.name == "listen").unwrap();
        assert!(listen.ty.contains("SocketAddr"));
        assert!(listen.default.contains("1079"));
    }

    #[test]
    fn builds_page_with_overrides() {
        let effective = serde_json::json!({
            "listen": "[::]:1079",
            "asn": 65001,
            "tls": { "root_cafile_path": "/etc/ca.crt" },
        });
        let mut explicit = BTreeMap::new();
        explicit.insert("asn".to_string(), "site.toml".to_string());
        explicit.insert("tls.root_cafile_path".to_string(), "base.toml".to_string());

        let page = build_config_page(
            carbide_api_core::cfg::CONFIG_REFERENCE_MD,
            &effective,
            &explicit,
            Vec::new(),
        );
        let rows: Vec<&ConfigRowView> = page
            .groups
            .iter()
            .flat_map(|g| g.sections.iter())
            .flat_map(|s| s.rows.iter())
            .collect();
        assert!(rows.len() > 150, "expected full catalog, got {}", rows.len());
        let asn = rows.iter().find(|r| r.path == "asn").unwrap();
        assert!(asn.overridden);
        assert_eq!(asn.source, "site.toml");
        assert!(asn.value_html.contains("65001"));
        let listen = rows.iter().find(|r| r.path == "listen").unwrap();
        assert!(!listen.overridden);
        // Nested section rows exist with dotted paths.
        assert!(rows.iter().any(|r| r.path == "tls.root_cafile_path"));
        // A field whose sub-struct was set is marked overridden by prefix.
        let tls_row = rows.iter().find(|r| r.path == "tls.root_cafile_path").unwrap();
        assert!(tls_row.overridden);
    }

    #[test]
    fn markdown_lite_renders_and_escapes() {
        assert_eq!(
            markdown_lite("Use `ip_address` for <new> hosts"),
            "Use <code>ip_address</code> for &lt;new&gt; hosts"
        );
        assert_eq!(markdown_lite("**Deprecated.**"), "<strong>Deprecated.</strong>");
        assert_eq!(
            markdown_lite("see [SiteExplorerConfig](#siteexplorerconfig)."),
            "see SiteExplorerConfig."
        );
    }

    #[test]
    fn duration_and_collections_format() {
        let (html, multiline) = format_value(&serde_json::json!({"secs": 3600, "nanos": 0}));
        assert_eq!(html, "<code>1h</code>");
        assert!(!multiline);
        let (html, _) = format_value(&serde_json::json!(["10.0.0.1", "10.0.0.2"]));
        assert!(html.contains("10.0.0.1, 10.0.0.2"));
        let (_, multiline) = format_value(&serde_json::json!({"a": {"b": 1}}));
        assert!(multiline);
    }
}
