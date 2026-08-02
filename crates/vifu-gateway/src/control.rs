use url::Url;
use uuid::Uuid;
use vifu_runtime::{RuntimeManifest, RuntimeRelease, RuntimeTraceRecord};

use serde::{Deserialize, Serialize};

use crate::relay::agent_gateway_websocket_url;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestProjectBootstrap {
    pub project: GuestProject,
    pub deployment: GuestDeployment,
    pub endpoint_path: String,
    pub api_key: String,
    pub claim_token: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuestProject {
    pub id: Uuid,
    pub slug: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuestDeployment {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayRuntimeConfiguration {
    pub gateway_id: String,
    pub deployments: Vec<RuntimeDeploymentConfiguration>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDeploymentConfiguration {
    pub deployment_id: Uuid,
    pub deployment: String,
    pub project_id: Uuid,
    pub project_slug: String,
    pub project_name: String,
    pub is_primary: bool,
    pub policies: RuntimeDeploymentPolicies,
    pub release: Option<RuntimeRelease>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDeploymentPolicies {
    pub config_sync: bool,
    pub trace_mode: String,
    pub remote_invocation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadedRuntimeTraces {
    accepted_trace_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadRuntimeTraces<'a> {
    deployment_id: Uuid,
    traces: &'a [RuntimeTraceRecord],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapRuntimeRelease<'a> {
    deployment_id: Uuid,
    manifest: &'a RuntimeManifest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapRuntimeReleaseResponse {
    release: PublishedRuntimeRelease,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedRuntimeRelease {
    version: u64,
    content_hash: String,
    manifest: RuntimeManifest,
}

#[derive(Clone)]
pub struct RuntimeControlClient {
    client: reqwest::Client,
    config_url: Url,
    traces_url: Url,
    release_url: Url,
    credential: String,
}

impl RuntimeControlClient {
    pub fn new(server_url: &str, credential: impl Into<String>) -> Result<Self, String> {
        agent_gateway_websocket_url(server_url)?;
        let config_url = endpoint_url(server_url, "runtime-config")?;
        let traces_url = endpoint_url(server_url, "runtime-traces")?;
        let release_url = endpoint_url(server_url, "runtime-releases/bootstrap")?;
        Ok(Self {
            client: reqwest::Client::new(),
            config_url,
            traces_url,
            release_url,
            credential: credential.into(),
        })
    }

    pub async fn configuration(&self) -> Result<GatewayRuntimeConfiguration, String> {
        let response = self
            .client
            .get(self.config_url.clone())
            .bearer_auth(&self.credential)
            .send()
            .await
            .map_err(|error| format!("runtime configuration request failed: {error}"))?;
        decode_response(response, "runtime configuration").await
    }

    pub async fn bootstrap_guest_project(
        server_url: &str,
        credential: &str,
    ) -> Result<GuestProjectBootstrap, String> {
        let url = server_endpoint_url(server_url, "v1/guest/bootstrap")?;
        let response = reqwest::Client::new()
            .post(url)
            .bearer_auth(credential)
            .send()
            .await
            .map_err(|error| format!("guest project request failed: {error}"))?;
        decode_response(response, "guest project bootstrap").await
    }

    pub async fn upload_traces(
        &self,
        deployment_id: Uuid,
        traces: &[RuntimeTraceRecord],
    ) -> Result<Vec<String>, String> {
        if traces.is_empty() || traces.len() > 100 {
            return Err("runtime trace batches must contain between 1 and 100 records".to_string());
        }
        for trace in traces {
            trace.validate().map_err(|error| error.to_string())?;
        }
        let response = self
            .client
            .post(self.traces_url.clone())
            .bearer_auth(&self.credential)
            .json(&UploadRuntimeTraces {
                deployment_id,
                traces,
            })
            .send()
            .await
            .map_err(|error| format!("runtime trace upload failed: {error}"))?;
        let response =
            decode_response::<UploadedRuntimeTraces>(response, "runtime trace upload").await?;
        Ok(response.accepted_trace_ids)
    }

    pub async fn bootstrap_runtime_release(
        &self,
        deployment_id: Uuid,
        manifest: &RuntimeManifest,
    ) -> Result<RuntimeRelease, String> {
        manifest.validate().map_err(|error| error.to_string())?;
        let response = self
            .client
            .post(self.release_url.clone())
            .bearer_auth(&self.credential)
            .json(&BootstrapRuntimeRelease {
                deployment_id,
                manifest,
            })
            .send()
            .await
            .map_err(|error| format!("runtime release bootstrap failed: {error}"))?;
        let response = decode_response::<BootstrapRuntimeReleaseResponse>(
            response,
            "runtime release bootstrap",
        )
        .await?;
        let release = RuntimeRelease {
            version: response.release.version,
            content_hash: response.release.content_hash,
            manifest: response.release.manifest,
        };
        release.validate().map_err(|error| error.to_string())?;
        Ok(release)
    }
}

async fn decode_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    operation: &str,
) -> Result<T, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "server rejected {operation} (HTTP {})",
            status.as_u16()
        ));
    }
    response
        .json::<T>()
        .await
        .map_err(|error| format!("server returned invalid {operation}: {error}"))
}

fn endpoint_url(server_url: &str, endpoint: &str) -> Result<Url, String> {
    server_endpoint_url(server_url, &format!("v1/agent-gateway/{endpoint}"))
}

fn server_endpoint_url(server_url: &str, endpoint: &str) -> Result<Url, String> {
    let _ = agent_gateway_websocket_url(server_url)?;
    let mut url = Url::parse(server_url.trim())
        .map_err(|_| "gateway.serverUrl must be a valid HTTP or HTTPS URL".to_string())?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/{}", endpoint.trim_start_matches('/')));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_control_urls_beside_the_gateway_websocket() {
        assert_eq!(
            endpoint_url("https://runtime.example.com/api/", "runtime-config")
                .unwrap()
                .as_str(),
            "https://runtime.example.com/api/v1/agent-gateway/runtime-config"
        );
        assert_eq!(
            server_endpoint_url("https://runtime.example.com/api/", "v1/guest/bootstrap")
                .unwrap()
                .as_str(),
            "https://runtime.example.com/api/v1/guest/bootstrap"
        );
    }

    #[test]
    fn parses_a_runtime_configuration() {
        let configuration =
            serde_json::from_value::<GatewayRuntimeConfiguration>(serde_json::json!({
                "gatewayId": "gateway-1",
                "deployments": [{
                    "deploymentId": Uuid::nil(),
                    "deployment": "development",
                    "projectId": Uuid::nil(),
                    "projectSlug": "moon-train",
                    "projectName": "Moon Train",
                    "isPrimary": true,
                    "policies": {
                        "configSync": true,
                        "traceMode": "summary",
                        "remoteInvocation": false
                    },
                    "release": null
                }]
            }))
            .unwrap();
        assert_eq!(configuration.deployments[0].project_slug, "moon-train");
    }

    #[test]
    fn parses_guest_bootstrap_with_expanded_server_resources() {
        let project_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let bootstrap = serde_json::from_value::<GuestProjectBootstrap>(serde_json::json!({
            "project": {
                "id": project_id,
                "slug": "guest-example",
                "name": "Guest project",
                "gatewayId": "gateway-example",
                "bindings": []
            },
            "deployment": {
                "id": deployment_id,
                "projectId": project_id,
                "name": "development",
                "isPrimary": true,
                "configSync": false,
                "traceMode": "summary",
                "remoteInvocation": false
            },
            "endpointPath": "/guest-example/v1",
            "apiKey": "vifu_pk_example",
            "claimToken": "vifu_gc_example",
            "expiresAt": "2026-08-08T00:00:00Z"
        }))
        .unwrap();

        assert_eq!(bootstrap.project.id, project_id);
        assert_eq!(bootstrap.deployment.id, deployment_id);
        assert_eq!(bootstrap.endpoint_path, "/guest-example/v1");
    }
}
