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

//! Explicit plugin registry and deterministic selection.

use std::sync::Arc;

use carbide_redfish_platform_api::error::RedfishError;
use carbide_redfish_platform_api::model::{MatchSpecificity, PlatformIdentity, PluginId};
use carbide_redfish_platform_api::plugin::PlatformPlugin;
use carbide_redfish_platform_api::registry::PluginRegistrar;

/// Builder that collects plugins via explicit registration.
#[derive(Default)]
pub struct PlatformRegistryBuilder {
    plugins: Vec<Arc<dyn PlatformPlugin>>,
}

impl PlatformRegistryBuilder {
    /// New empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Finalize into an immutable registry.
    pub fn build(self) -> PlatformRegistry {
        PlatformRegistry {
            plugins: self.plugins,
        }
    }
}

impl PluginRegistrar for PlatformRegistryBuilder {
    fn register(&mut self, plugin: Arc<dyn PlatformPlugin>) {
        self.plugins.push(plugin);
    }
}

/// Immutable set of registered plugins.
pub struct PlatformRegistry {
    plugins: Vec<Arc<dyn PlatformPlugin>>,
}

impl PlatformRegistry {
    /// Select the most specific plugin for an identity. Ties at the top
    /// specificity fail closed.
    pub fn select(
        &self,
        identity: &PlatformIdentity,
    ) -> Result<(Arc<dyn PlatformPlugin>, MatchSpecificity), RedfishError> {
        let matches: Vec<(MatchSpecificity, &Arc<dyn PlatformPlugin>)> = self
            .plugins
            .iter()
            .filter_map(|p| p.detect(identity).map(|spec| (spec, p)))
            .collect();

        let Some(max) = matches.iter().map(|(spec, _)| *spec).max() else {
            return Err(RedfishError::NoMatchingPlugin);
        };

        let top: Vec<&Arc<dyn PlatformPlugin>> = matches
            .iter()
            .filter(|(spec, _)| *spec == max)
            .map(|(_, p)| *p)
            .collect();

        match top.as_slice() {
            [only] => Ok(((*only).clone(), max)),
            _ => Err(RedfishError::AmbiguousSelection(
                top.iter().map(|p| p.metadata().id.to_string()).collect(),
            )),
        }
    }

    /// Find a registered plugin by id.
    pub fn find_by_id(&self, id: &PluginId) -> Option<Arc<dyn PlatformPlugin>> {
        self.plugins
            .iter()
            .find(|p| &p.metadata().id == id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carbide_redfish_platform_api::model::{PlatformMetadata, SupportLevel};

    struct MockPlugin {
        metadata: PlatformMetadata,
        spec: Option<MatchSpecificity>,
    }

    impl PlatformPlugin for MockPlugin {
        fn metadata(&self) -> &PlatformMetadata {
            &self.metadata
        }
        fn detect(&self, _identity: &PlatformIdentity) -> Option<MatchSpecificity> {
            self.spec
        }
    }

    fn mock(id: &str, spec: Option<MatchSpecificity>) -> Arc<dyn PlatformPlugin> {
        Arc::new(MockPlugin {
            metadata: PlatformMetadata {
                id: PluginId(id.to_string()),
                vendor: "mock",
                models: &[],
                plugin_version: "0.0.0",
                support_level: SupportLevel::Candidate,
            },
            spec,
        })
    }

    fn registry(plugins: Vec<Arc<dyn PlatformPlugin>>) -> PlatformRegistry {
        let mut builder = PlatformRegistryBuilder::new();
        for p in plugins {
            builder.register(p);
        }
        builder.build()
    }

    #[test]
    fn most_specific_match_wins_over_generic() {
        let reg = registry(vec![
            mock("standard", Some(MatchSpecificity::Generic)),
            mock("vendor", Some(MatchSpecificity::Vendor)),
        ]);
        let (selected, spec) = reg.select(&PlatformIdentity::default()).unwrap();
        assert_eq!(selected.metadata().id, PluginId("vendor".to_string()));
        assert_eq!(spec, MatchSpecificity::Vendor);
    }

    #[test]
    fn model_outranks_vendor() {
        let reg = registry(vec![
            mock("standard", Some(MatchSpecificity::Generic)),
            mock("vendor", Some(MatchSpecificity::Vendor)),
            mock("model", Some(MatchSpecificity::Model)),
        ]);
        let (selected, spec) = reg.select(&PlatformIdentity::default()).unwrap();
        assert_eq!(selected.metadata().id, PluginId("model".to_string()));
        assert_eq!(spec, MatchSpecificity::Model);
    }

    #[test]
    fn tie_at_top_specificity_fails_closed() {
        let reg = registry(vec![
            mock("model-a", Some(MatchSpecificity::Model)),
            mock("model-b", Some(MatchSpecificity::Model)),
        ]);
        assert!(matches!(
            reg.select(&PlatformIdentity::default()),
            Err(RedfishError::AmbiguousSelection(_))
        ));
    }

    #[test]
    fn no_match_errors() {
        let reg = registry(vec![mock("none", None)]);
        assert!(matches!(
            reg.select(&PlatformIdentity::default()),
            Err(RedfishError::NoMatchingPlugin)
        ));
    }

    #[test]
    fn find_by_id_returns_registered_plugin() {
        let reg = registry(vec![mock("standard", Some(MatchSpecificity::Generic))]);
        assert!(reg.find_by_id(&PluginId("standard".to_string())).is_some());
        assert!(reg.find_by_id(&PluginId("missing".to_string())).is_none());
    }
}
