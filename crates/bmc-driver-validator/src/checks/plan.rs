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

//! Which checks a run plans, and which of them cannot run before any does.

use bmc_platform::Capability;

use super::model::{Check, Inputs, Kind, Outcome};
use super::validate::{self, EndState};
use super::{apply, reads};
use crate::selection::Selection;

pub(crate) struct Planned {
    pub(crate) check: Check,
    /// Why the check will not run, decided before any check runs.
    pub(crate) skip: Option<Outcome>,
}

/// Which checks a run plans.
pub(crate) enum Scope<'a> {
    /// Every read check, plus the apply checks with `apply`, narrowed to the
    /// capabilities in `only` and the checks in `ids`.
    Checks {
        apply: bool,
        only: &'a [Capability],
        ids: &'a [String],
    },
    /// The `--validate` sequence.
    Validate(EndState),
    /// The factory reset `--factory-reset-first` runs before `--validate`.
    FactoryReset,
}

pub(crate) fn plan(
    selection: &Selection,
    inputs: &Inputs,
    scope: Scope<'_>,
) -> Result<Vec<Planned>, String> {
    let checks = match scope {
        Scope::Checks { apply, only, ids } => chosen(apply, only, ids)?,
        Scope::Validate(end_state) => {
            validate::checks(selection.is_supported(Capability::Dpu), end_state)
        }
        Scope::FactoryReset => validate::factory_reset_first(),
    };
    Ok(checks
        .into_iter()
        .map(|check| {
            let skip = if !selection.is_supported(check.capability) {
                Some(Outcome::Unsupported(selection.reason(check.capability)))
            } else {
                check
                    .needs
                    .iter()
                    .find(|needs| !inputs.has(**needs))
                    .map(|needs| needs.skipped())
            };
            Planned { check, skip }
        })
        .collect())
}

fn chosen(apply: bool, only: &[Capability], ids: &[String]) -> Result<Vec<Check>, String> {
    let all: Vec<Check> = reads::checks().into_iter().chain(apply::checks()).collect();
    for id in ids {
        let check = all
            .iter()
            .find(|check| check.id.as_str() == id)
            .ok_or_else(|| {
                let known: Vec<&str> = all.iter().map(|check| check.id.as_str()).collect();
                format!("unknown check {id}; known checks: {}", known.join(", "))
            })?;
        if check.kind == Kind::Apply && !apply {
            return Err(format!("{id} changes the BMC; pass --apply to run it"));
        }
    }
    let narrowed = !only.is_empty() || !ids.is_empty();
    Ok(all
        .into_iter()
        .filter(|check| check.kind == Kind::Read || apply)
        .filter(|check| {
            !narrowed
                || only.contains(&check.capability)
                || ids.iter().any(|id| id == check.id.as_str())
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--only` and `--check` narrow the default set together, and a check
    /// that changes the BMC is refused unless `--apply` is given.
    #[test]
    fn checks_are_chosen_by_capability_id_and_apply() {
        let ids = |checks: Vec<Check>| -> Vec<&'static str> {
            checks.iter().map(|check| check.id.as_str()).collect()
        };
        let reads_only = ids(chosen(false, &[], &[]).expect("chosen"));
        assert!(!reads_only.is_empty());
        assert!(reads_only.iter().all(|id| !id.ends_with(".apply")));
        assert!(ids(chosen(true, &[], &[]).expect("chosen")).contains(&"bios.apply"));

        let narrowed = ids(chosen(
            false,
            &[Capability::Power],
            &["lockdown.status".to_string()],
        )
        .expect("chosen"));
        assert_eq!(
            narrowed,
            [
                "power.state",
                "power.ac_power_cycle_supported",
                "lockdown.status"
            ]
        );

        let refused = [
            ("unknown", "no.such_check", "unknown check no.such_check"),
            ("apply without --apply", "bios.apply", "pass --apply"),
        ];
        for (scenario, id, error) in refused {
            let result = chosen(false, &[], &[id.to_string()]).map(ids);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|message| message.contains(error)),
                "{scenario}: {result:?}"
            );
        }
    }
}
