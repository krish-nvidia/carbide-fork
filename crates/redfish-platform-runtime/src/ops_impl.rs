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

//! The `nv-redfish` implementation of the [`RedfishOps`] façade.
//!
//! All transport, ETag, `@odata.id`, and action plumbing lives here, so plugins
//! work purely in terms of Redfish paths and JSON.

use std::sync::Arc;

use async_trait::async_trait;
use carbide_redfish_platform_api::error::RedfishError;
use carbide_redfish_platform_api::model::PlatformIdentity;
use carbide_redfish_platform_api::ops::RedfishOps;
use nv_redfish::Bmc as _;
use nv_redfish::Resource as _;
use nv_redfish::ServiceRoot;
use nv_redfish::bmc_http::reqwest::{Client as NvClient, ClientParams};
use nv_redfish::bmc_http::{BmcCredentials, CacheSettings, HttpBmc};
use nv_redfish::core::{Action, EntityTypeRef, ModificationResponse, ODataETag, ODataId};
use nv_redfish::session_service::{Session, SessionCreate};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::AuthMode;

/// The concrete BMC backend used everywhere in NICo.
pub type RedfishBmc = HttpBmc<NvClient>;

/// A minimal entity wrapper used to GET/DELETE arbitrary JSON resources (the
/// `Bmc` trait requires the type to implement `EntityTypeRef`).
#[derive(Debug, Deserialize)]
struct RawJson {
    #[serde(rename = "@odata.id", default = "ODataId::service_root")]
    odata_id: ODataId,
    #[serde(rename = "@odata.etag", default)]
    etag: Option<ODataETag>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl EntityTypeRef for RawJson {
    fn odata_id(&self) -> &ODataId {
        &self.odata_id
    }
    fn etag(&self) -> Option<&ODataETag> {
        self.etag.as_ref()
    }
}

/// Map an `nv-redfish` error into the shared [`RedfishError`].
pub fn map_nv_error(err: nv_redfish::Error<RedfishBmc>) -> RedfishError {
    use nv_redfish::bmc_http::reqwest::BmcError;

    match err {
        nv_redfish::Error::Bmc(bmc_err) => match bmc_err {
            BmcError::InvalidResponse { url, status, text } => RedfishError::HttpStatus {
                url: url.to_string(),
                status_code: status.as_u16(),
                response_body: text,
            },
            BmcError::ReqwestError(e) => RedfishError::Network {
                url: e
                    .url()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<unknown>".to_string()),
                source: Box::new(e),
            },
            BmcError::JsonError(e) => RedfishError::Deserialize {
                url: "<unknown>".to_string(),
                source: Box::new(e),
            },
            BmcError::DecodeError(e) => RedfishError::Deserialize {
                url: "<unknown>".to_string(),
                source: Box::new(e),
            },
            other => RedfishError::Generic(format!("bmc transport error: {other}")),
        },
        nv_redfish::Error::ActionNotAvailable => {
            RedfishError::NotSupported("redfish action not available".to_string())
        }
        nv_redfish::Error::Json(e) => RedfishError::Deserialize {
            url: "<unknown>".to_string(),
            source: Box::new(e),
        },
        other => RedfishError::Generic(format!("nv-redfish error: {other}")),
    }
}

/// An authenticated connection to one BMC plus its service root, exposed to
/// plugins through the [`RedfishOps`] façade.
pub struct NvRedfishOps {
    bmc: Arc<RedfishBmc>,
    root: ServiceRoot<RedfishBmc>,
    /// Present only in [`AuthMode::Session`]; deleted best-effort on drop.
    session: Option<Session<RedfishBmc>>,
}

impl NvRedfishOps {
    /// Build a client to `https://{address}`, authenticate per `auth_mode`, and
    /// fetch the service root.
    pub async fn connect(
        address: std::net::SocketAddr,
        username: String,
        password: String,
        auth_mode: AuthMode,
    ) -> Result<Self, RedfishError> {
        // BMCs (notably HPE iLO) silently drop kept-alive connections, so
        // reusing a pooled socket races with a server-side close and surfaces
        // as `hyper IncompleteMessage`. Disable idle-connection pooling so each
        // request uses a fresh connection -- BMC traffic is low-volume, and this
        // avoids the connection-reuse RST without per-vendor handling.
        let client = NvClient::with_params(
            ClientParams::new()
                .accept_invalid_certs(true)
                .pool_max_idle_per_host(0),
        )
        .map_err(|e| RedfishError::Network {
            url: address.to_string(),
            source: Box::new(e),
        })?;

        let url = Url::parse(&format!("https://{address}"))
            .map_err(|e| RedfishError::generic(format!("invalid BMC URL: {e}")))?;

        let bmc = Arc::new(HttpBmc::new(
            client,
            url,
            BmcCredentials::new(username.clone(), password.clone()),
            CacheSettings::with_capacity(10),
        ));

        let root = ServiceRoot::new(bmc.clone()).await.map_err(map_nv_error)?;

        let session = match auth_mode {
            AuthMode::BasicAuth => None,
            AuthMode::Session => Some(establish_session(&bmc, username, password).await?),
        };

        // In session mode the credentials swapped to a token, so re-fetch the
        // service root under the session token.
        let root = if session.is_some() {
            ServiceRoot::new(bmc.clone()).await.map_err(map_nv_error)?
        } else {
            root
        };

        Ok(Self { bmc, root, session })
    }

    /// Anonymous reachability probe: fetch the service root (which the Redfish
    /// spec requires to be readable without auth) with no credentials. Success
    /// means a Redfish service is answering at the address.
    pub async fn probe(address: std::net::SocketAddr) -> Result<(), RedfishError> {
        let client = NvClient::with_params(
            ClientParams::new()
                .accept_invalid_certs(true)
                .pool_max_idle_per_host(0),
        )
        .map_err(|e| RedfishError::Network {
            url: address.to_string(),
            source: Box::new(e),
        })?;
        let url = Url::parse(&format!("https://{address}"))
            .map_err(|e| RedfishError::generic(format!("invalid BMC URL: {e}")))?;
        let bmc = Arc::new(HttpBmc::new(
            client,
            url,
            BmcCredentials::new(String::new(), String::new()),
            CacheSettings::with_capacity(1),
        ));
        ServiceRoot::new(bmc).await.map_err(map_nv_error)?;
        Ok(())
    }

    /// Identity from data already in hand after `connect` (vendor/product from
    /// the service root) -- no extra BMC reads. Used on the hinted fast path,
    /// where a full identity gather is unnecessary.
    pub fn cheap_identity(&self) -> PlatformIdentity {
        PlatformIdentity {
            service_root_vendor: self.root.vendor().map(|v| v.to_string()),
            service_root_product: self.root.product().map(|p| p.to_string()),
            ..PlatformIdentity::default()
        }
    }

    /// Collect the cheap evidence used for plugin selection.
    pub async fn gather_identity(&self) -> Result<PlatformIdentity, RedfishError> {
        let mut identity = PlatformIdentity {
            service_root_vendor: self.root.vendor().map(|v| v.to_string()),
            service_root_product: self.root.product().map(|p| p.to_string()),
            ..PlatformIdentity::default()
        };

        // OEM keys are a first-class selection signal (e.g. Lenovo-AMI keys off
        // the "Ami" OEM key), so read them from the raw service root using the
        // shared extractor (kept identical to site exploration's hint).
        if let Ok(root) = self.get("/redfish/v1").await {
            identity.service_root_oem_keys =
                carbide_redfish_platform_api::model::service_root_oem_keys(&root);
        }

        // Manager/system/chassis reads are best-effort: factory-state BMCs
        // (e.g. NVIDIA GBx00) answer the service root but return 403
        // `PasswordChangeRequired` on /Systems until the password is rotated.
        // Selection must still work from the service-root evidence so the
        // rotation itself can go through a plugin.
        if let Ok(Some(managers)) = self.root.managers().await
            && let Ok(members) = managers.members().await
            && let Some(manager) = members.into_iter().next()
        {
            identity.manager_id = Some(manager.id().to_string());
            identity.manager_firmware_version = manager.raw().firmware_version.clone().flatten();
        }

        if let Ok(Some(systems)) = self.root.systems().await
            && let Ok(members) = systems.members().await
            && let Some(system) = members.into_iter().next()
        {
            identity.system_id = Some(system.id().to_string());
            let hw = system.hardware_id();
            identity.system_manufacturer = hw.manufacturer.map(|m| m.to_string());
            identity.system_model = hw.model.map(|m| m.to_string());
        }

        if let Ok(Some(chassis)) = self.root.chassis().await
            && let Ok(members) = chassis.members().await
        {
            identity.chassis_ids = members.iter().map(|c| c.id().to_string()).collect();
        }

        Ok(identity)
    }
}

/// Create a Redfish session and swap the client onto the returned token.
async fn establish_session(
    bmc: &Arc<RedfishBmc>,
    username: String,
    password: String,
) -> Result<Session<RedfishBmc>, RedfishError> {
    let root = ServiceRoot::new(bmc.clone()).await.map_err(map_nv_error)?;
    let service = root
        .session_service()
        .await
        .map_err(map_nv_error)?
        .ok_or_else(|| RedfishError::not_supported("BMC does not expose SessionService"))?;
    let sessions = service
        .sessions()
        .await
        .map_err(map_nv_error)?
        .ok_or_else(|| RedfishError::not_supported("SessionService has no Sessions collection"))?;
    let session = sessions
        .create_session(&SessionCreate::builder(username, password).build())
        .await
        .map_err(map_nv_error)?;
    let token = session
        .auth_token()
        .ok_or_else(|| RedfishError::generic("session response had no X-Auth-Token"))?
        .to_string();
    bmc.set_credentials(BmcCredentials::token(token));
    Ok(session)
}

impl Drop for NvRedfishOps {
    fn drop(&mut self) {
        // Best-effort session logout. Drop cannot be async, so spawn the delete
        // when a Tokio runtime is available; otherwise rely on BMC expiry.
        if let Some(session) = self.session.take()
            && let Ok(handle) = tokio::runtime::Handle::try_current()
        {
            handle.spawn(async move {
                let _ = session.delete().await;
            });
        }
    }
}

#[async_trait]
impl RedfishOps for NvRedfishOps {
    async fn get(&self, path: &str) -> Result<Value, RedfishError> {
        let raw = self
            .bmc
            .as_ref()
            .get::<RawJson>(&ODataId::from(path.to_string()))
            .await
            .map_err(|e| map_nv_error(nv_redfish::Error::Bmc(e)))?;
        let mut map = raw.rest.clone();
        map.insert("@odata.id".to_string(), json!(raw.odata_id.to_string()));
        Ok(Value::Object(map))
    }

    async fn patch(&self, path: &str, body: Value) -> Result<(), RedfishError> {
        self.bmc
            .as_ref()
            .update::<Value, Value>(&ODataId::from(path.to_string()), None, &body)
            .await
            .map_err(|e| map_nv_error(nv_redfish::Error::Bmc(e)))?;
        Ok(())
    }

    async fn post_action(&self, path: &str, body: Value) -> Result<Value, RedfishError> {
        // `Action` only deserializes its `target` and has no public constructor,
        // so build one from JSON. Deliberate -- not a candidate for "cleanup".
        let action: Action<Value, Value> = serde_json::from_value(json!({ "target": path }))
            .map_err(|e| RedfishError::generic(format!("constructing action target: {e}")))?;
        let resp = action
            .run(self.bmc.as_ref(), &body)
            .await
            .map_err(|e| map_nv_error(nv_redfish::Error::Bmc(e)))?;
        Ok(match resp {
            ModificationResponse::Entity(v) => v,
            _ => Value::Null,
        })
    }

    async fn create(&self, path: &str, body: Value) -> Result<Value, RedfishError> {
        let resp = self
            .bmc
            .as_ref()
            .create::<Value, Value>(&ODataId::from(path.to_string()), &body)
            .await
            .map_err(|e| map_nv_error(nv_redfish::Error::Bmc(e)))?;
        Ok(match resp {
            ModificationResponse::Entity(v) => v,
            _ => Value::Null,
        })
    }

    async fn delete(&self, path: &str) -> Result<(), RedfishError> {
        self.bmc
            .as_ref()
            .delete::<RawJson>(&ODataId::from(path.to_string()))
            .await
            .map_err(|e| map_nv_error(nv_redfish::Error::Bmc(e)))?;
        Ok(())
    }

    async fn system_path(&self) -> Result<String, RedfishError> {
        let systems = self
            .root
            .systems()
            .await
            .map_err(map_nv_error)?
            .ok_or_else(|| RedfishError::not_supported("Systems collection not present"))?;
        let system = systems
            .members()
            .await
            .map_err(map_nv_error)?
            .into_iter()
            .next()
            .ok_or_else(|| RedfishError::generic("no ComputerSystem members present"))?;
        Ok(system.odata_id().to_string())
    }

    async fn manager_path(&self) -> Result<String, RedfishError> {
        let managers = self
            .root
            .managers()
            .await
            .map_err(map_nv_error)?
            .ok_or_else(|| RedfishError::not_supported("Managers collection not present"))?;
        let manager = managers
            .members()
            .await
            .map_err(map_nv_error)?
            .into_iter()
            .next()
            .ok_or_else(|| RedfishError::generic("no Manager members present"))?;
        Ok(manager.odata_id().to_string())
    }
}
