use std::fmt;
use std::time::Duration;

use crate::protocol::TraceTelemetryBatch;
use reqwest::StatusCode;
use url::Url;
use uuid::Uuid;
use vifu_runtime::{RuntimeManifest, RuntimeRelease, RuntimeTraceRecord};

use serde::{Deserialize, Serialize};

use crate::optimization::RuntimeComparisonUpload;
use crate::relay::agent_gateway_websocket_url;

const TRACE_OBSERVATION_UPLOAD_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TraceObservationUploadError {
    Retryable(String),
    Permanent(String),
}

impl TraceObservationUploadError {
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }
}

impl fmt::Display for TraceObservationUploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable(message) | Self::Permanent(message) => formatter.write_str(message),
        }
    }
}

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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestGatewayEnrollment {
    #[serde(default)]
    pub enrollment_id: Option<Uuid>,
    pub enrollment_token: String,
    pub expires_at: String,
    pub deployment: String,
    pub pairing: Option<GatewayPairing>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayPairing {
    pub server_url: String,
    pub pairing_uri: String,
    pub pairing_deep_link: String,
    pub pairing_terminal_qr: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ServerStatus {
    #[serde(default)]
    dashboard_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayRuntimeConfiguration {
    pub gateway_id: String,
    pub deployments: Vec<RuntimeDeploymentConfiguration>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayRuntimeAgents {
    pub gateway_id: String,
    pub deployments: Vec<RuntimeDeploymentAgents>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDeploymentAgents {
    pub deployment_id: Uuid,
    pub deployment: String,
    pub project_id: Uuid,
    pub project_slug: String,
    pub project_name: String,
    pub is_primary: bool,
    pub agents: Vec<RuntimeProjectAgent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeProjectAgent {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub capabilities: Vec<String>,
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
    #[serde(default)]
    pub binding_ids: Vec<Uuid>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadedRuntimeComparison {
    comparison_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadedRuntimeTraceObservations {
    accepted_request_id: Uuid,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportRuntimeReleaseApplied<'a> {
    deployment_id: Uuid,
    release_version: u64,
    content_hash: &'a str,
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
    agents_url: Url,
    traces_url: Url,
    trace_observations_url: Url,
    comparisons_url: Url,
    release_url: Url,
    release_applied_url: Url,
    credential: String,
}

impl RuntimeControlClient {
    pub async fn discover_dashboard_url(server_url: &str) -> Result<Option<String>, String> {
        let url = server_endpoint_url(server_url, "v1/status")?;
        let response = http_client(None)?
            .get(url)
            .send()
            .await
            .map_err(|error| format!("server status request failed: {error}"))?;
        let dashboard_url = decode_response::<ServerStatus>(response, "server status")
            .await?
            .dashboard_url;
        dashboard_url.map(validate_dashboard_url).transpose()
    }

    pub fn new(server_url: &str, credential: impl Into<String>) -> Result<Self, String> {
        Self::new_with_server_certificate(server_url, credential, None)
    }

    pub fn new_with_server_certificate(
        server_url: &str,
        credential: impl Into<String>,
        server_certificate_der: Option<&[u8]>,
    ) -> Result<Self, String> {
        agent_gateway_websocket_url(server_url)?;
        let config_url = endpoint_url(server_url, "runtime-config")?;
        let agents_url = endpoint_url(server_url, "runtime-agents")?;
        let traces_url = endpoint_url(server_url, "runtime-traces")?;
        let trace_observations_url = endpoint_url(server_url, "runtime-trace-observations")?;
        let comparisons_url = endpoint_url(server_url, "runtime-comparisons")?;
        let release_url = endpoint_url(server_url, "runtime-releases/bootstrap")?;
        let release_applied_url = endpoint_url(server_url, "runtime-releases/applied")?;
        Ok(Self {
            client: http_client(server_certificate_der)?,
            config_url,
            agents_url,
            traces_url,
            trace_observations_url,
            comparisons_url,
            release_url,
            release_applied_url,
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

    pub async fn report_release_applied(
        &self,
        deployment_id: Uuid,
        release: &RuntimeRelease,
    ) -> Result<(), String> {
        let response = self
            .client
            .post(self.release_applied_url.clone())
            .bearer_auth(&self.credential)
            .json(&ReportRuntimeReleaseApplied {
                deployment_id,
                release_version: release.version,
                content_hash: &release.content_hash,
            })
            .send()
            .await
            .map_err(|error| format!("runtime release apply acknowledgement failed: {error}"))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        decode_response::<serde_json::Value>(response, "runtime release apply acknowledgement")
            .await?;
        Ok(())
    }

    pub async fn runtime_agents(&self) -> Result<GatewayRuntimeAgents, String> {
        let response = self
            .client
            .get(self.agents_url.clone())
            .bearer_auth(&self.credential)
            .send()
            .await
            .map_err(|error| format!("runtime agent roster request failed: {error}"))?;
        decode_response(response, "runtime agent roster").await
    }

    pub async fn bootstrap_guest_project(
        server_url: &str,
        credential: &str,
    ) -> Result<GuestProjectBootstrap, String> {
        Self::bootstrap_guest_project_with_server_certificate(server_url, credential, None).await
    }

    pub async fn bootstrap_guest_project_with_server_certificate(
        server_url: &str,
        credential: &str,
        server_certificate_der: Option<&[u8]>,
    ) -> Result<GuestProjectBootstrap, String> {
        let url = server_endpoint_url(server_url, "v1/guest/bootstrap")?;
        let response = http_client(server_certificate_der)?
            .post(url)
            .bearer_auth(credential)
            .send()
            .await
            .map_err(|error| format!("guest project request failed: {error}"))?;
        decode_response(response, "guest project bootstrap").await
    }

    pub async fn create_guest_gateway_enrollment(
        server_url: &str,
        project_api_key: &str,
    ) -> Result<GuestGatewayEnrollment, String> {
        Self::create_guest_gateway_enrollment_with_server_certificate(
            server_url,
            project_api_key,
            None,
        )
        .await
    }

    pub async fn create_guest_gateway_enrollment_with_server_certificate(
        server_url: &str,
        project_api_key: &str,
        server_certificate_der: Option<&[u8]>,
    ) -> Result<GuestGatewayEnrollment, String> {
        let url = server_endpoint_url(server_url, "v1/guest/agent-gateway-enrollments")?;
        let response = http_client(server_certificate_der)?
            .post(url)
            .bearer_auth(project_api_key)
            .send()
            .await
            .map_err(|error| format!("Guest device enrollment request failed: {error}"))?;
        decode_response(response, "Guest device enrollment").await
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

    pub async fn upload_trace_observations(
        &self,
        request_id: Uuid,
        batch: &TraceTelemetryBatch,
    ) -> Result<(), String> {
        self.upload_trace_observations_classified(request_id, batch)
            .await
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn upload_trace_observations_classified(
        &self,
        request_id: Uuid,
        batch: &TraceTelemetryBatch,
    ) -> Result<(), TraceObservationUploadError> {
        crate::protocol::validate_trace_telemetry_batch(batch)
            .map_err(TraceObservationUploadError::Permanent)?;
        let response = self
            .client
            .post(self.trace_observations_url.clone())
            .bearer_auth(&self.credential)
            .timeout(TRACE_OBSERVATION_UPLOAD_TIMEOUT)
            .json(&serde_json::json!({
                "requestId": request_id,
                "events": batch.events,
                "droppedEvents": batch.dropped_events,
                "rootInputSummary": batch.root_input_summary,
                "rootOutputSummary": batch.root_output_summary,
            }))
            .send()
            .await
            .map_err(|error| {
                TraceObservationUploadError::Retryable(format!(
                    "runtime trace observation upload failed: {error}"
                ))
            })?;
        let status = response.status();
        if !status.is_success() {
            let message = format!(
                "server rejected runtime trace observation upload (HTTP {})",
                status.as_u16()
            );
            return Err(classify_trace_observation_status(status, message));
        }
        let response = response
            .json::<UploadedRuntimeTraceObservations>()
            .await
            .map_err(|error| {
                TraceObservationUploadError::Permanent(format!(
                    "server returned invalid runtime trace observation upload: {error}"
                ))
            })?;
        validate_trace_observation_ack(request_id, response)
            .map_err(TraceObservationUploadError::Permanent)
    }

    pub async fn upload_comparison(
        &self,
        comparison: &RuntimeComparisonUpload,
    ) -> Result<Uuid, String> {
        comparison.validate()?;
        let response = self
            .client
            .post(self.comparisons_url.clone())
            .bearer_auth(&self.credential)
            .json(comparison)
            .send()
            .await
            .map_err(|error| format!("runtime comparison upload failed: {error}"))?;
        let response =
            decode_response::<UploadedRuntimeComparison>(response, "runtime comparison upload")
                .await?;
        if response.comparison_id != comparison.id {
            return Err("server returned a different runtime comparison id".to_string());
        }
        Ok(response.comparison_id)
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

fn validate_dashboard_url(value: String) -> Result<String, String> {
    let url = Url::parse(value.trim())
        .map_err(|_| "server returned an invalid Dashboard URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("server returned an invalid Dashboard URL".to_string());
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn http_client(server_certificate_der: Option<&[u8]>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    if let Some(der) = server_certificate_der {
        if der.is_empty() {
            return Err("the pinned server certificate is empty".to_string());
        }
        let certificate = reqwest::Certificate::from_der(der)
            .map_err(|error| format!("the pinned server certificate is invalid: {error}"))?;
        builder = builder
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate);
    }
    builder
        .build()
        .map_err(|error| format!("the runtime control TLS client could not be created: {error}"))
}

fn classify_trace_observation_status(
    status: StatusCode,
    message: String,
) -> TraceObservationUploadError {
    if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        TraceObservationUploadError::Retryable(message)
    } else {
        TraceObservationUploadError::Permanent(message)
    }
}

fn validate_trace_observation_ack(
    request_id: Uuid,
    response: UploadedRuntimeTraceObservations,
) -> Result<(), String> {
    if response.accepted_request_id == request_id {
        Ok(())
    } else {
        Err("server acknowledged a different runtime trace observation request".to_string())
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
        .map_err(|_| "Vifu Server address must be a valid HTTP or HTTPS URL".to_string())?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/{}", endpoint.trim_start_matches('/')));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn builds_control_urls_beside_the_gateway_websocket() {
        assert_eq!(
            endpoint_url("https://runtime.example.com/api/", "runtime-config")
                .unwrap()
                .as_str(),
            "https://runtime.example.com/api/v1/agent-gateway/runtime-config"
        );
        assert_eq!(
            endpoint_url("https://runtime.example.com/api/", "runtime-comparisons")
                .unwrap()
                .as_str(),
            "https://runtime.example.com/api/v1/agent-gateway/runtime-comparisons"
        );
        assert_eq!(
            server_endpoint_url("https://runtime.example.com/api/", "v1/guest/bootstrap")
                .unwrap()
                .as_str(),
            "https://runtime.example.com/api/v1/guest/bootstrap"
        );
    }

    #[test]
    fn guest_enrollment_accepts_servers_from_before_enrollment_correlation() {
        let enrollment: GuestGatewayEnrollment = serde_json::from_value(serde_json::json!({
            "enrollmentToken": "vifu_ge_example",
            "expiresAt": "2026-08-08T00:00:00Z",
            "deployment": "development",
            "pairing": null
        }))
        .unwrap();

        assert_eq!(enrollment.enrollment_id, None);
    }

    #[test]
    fn server_status_accepts_a_separate_dashboard_origin() {
        let status: ServerStatus = serde_json::from_value(serde_json::json!({
            "service": "vifu-server",
            "status": "ok",
            "dashboardUrl": "https://dashboard.example.com"
        }))
        .unwrap();

        assert_eq!(
            status.dashboard_url.as_deref(),
            Some("https://dashboard.example.com")
        );
        assert_eq!(
            validate_dashboard_url(status.dashboard_url.unwrap()).unwrap(),
            "https://dashboard.example.com"
        );
    }

    #[test]
    fn dashboard_discovery_rejects_credentials() {
        assert!(validate_dashboard_url("https://user@example.com".to_string()).is_err());
    }

    #[tokio::test]
    async fn discovers_the_dashboard_from_server_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 2_048];
            let size = socket.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /v1/status "));
            let body = r#"{"dashboardUrl":"https://dashboard.example.com"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let dashboard = RuntimeControlClient::discover_dashboard_url(&format!("http://{address}"))
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(dashboard.as_deref(), Some("https://dashboard.example.com"));
    }

    #[test]
    fn trace_observation_ack_must_match_the_uploaded_request() {
        let request_id = Uuid::new_v4();
        assert!(validate_trace_observation_ack(
            request_id,
            UploadedRuntimeTraceObservations {
                accepted_request_id: request_id,
            },
        )
        .is_ok());
        assert!(validate_trace_observation_ack(
            request_id,
            UploadedRuntimeTraceObservations {
                accepted_request_id: Uuid::new_v4(),
            },
        )
        .unwrap_err()
        .contains("different"));
    }

    #[test]
    fn trace_observation_failures_only_retry_transient_statuses() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(classify_trace_observation_status(status, "failed".to_string()).is_retryable());
        }
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::PAYLOAD_TOO_LARGE,
        ] {
            assert!(
                !classify_trace_observation_status(status, "failed".to_string()).is_retryable()
            );
        }
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
                    "bindingIds": [],
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
    fn rejects_empty_or_invalid_pinned_server_certificates() {
        assert!(RuntimeControlClient::new_with_server_certificate(
            "https://macbook.local:6791",
            "device-token",
            Some(&[]),
        )
        .is_err());
        assert!(RuntimeControlClient::new_with_server_certificate(
            "https://macbook.local:6791",
            "device-token",
            Some(b"not-a-certificate"),
        )
        .is_err());
    }

    #[test]
    fn parses_a_runtime_agent_roster() {
        let profile_id = Uuid::new_v4();
        let roster = serde_json::from_value::<GatewayRuntimeAgents>(serde_json::json!({
            "gatewayId": "gateway-1",
            "deployments": [{
                "deploymentId": Uuid::nil(),
                "deployment": "development",
                "projectId": Uuid::nil(),
                "projectSlug": "stardew-valley",
                "projectName": "Stardew Valley",
                "isPrimary": true,
                "agents": [{
                    "id": profile_id,
                    "slug": "stardew-valley-farming-0",
                    "name": "Farming 0",
                    "capabilities": ["chat"]
                }]
            }]
        }))
        .unwrap();

        assert_eq!(
            roster.deployments[0].agents[0].slug,
            "stardew-valley-farming-0"
        );
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
