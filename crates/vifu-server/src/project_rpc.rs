use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_TYPE, HOST, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{future::join_all, SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

use crate::auth::{bearer_token, hash_api_key, is_hash_match};
use crate::db;
use crate::error::ApiError;
use crate::models::{ProjectAgentRoute, ProjectRoute};
use crate::relay::RelayCallError;
use crate::AppState;

const JSON_RPC_VERSION: &str = "2.0";
const JSON_RPC_PROTOCOL: &str = "jsonrpc";
const TOKEN_PROTOCOL_PREFIX: &str = "vifu.token.";
const MAX_RPC_MESSAGE_BYTES: usize = 512 * 1024;

pub async fn post_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    post(state, slug, headers, body).await
}

pub async fn post_by_host(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let slug = slug_from_host(&state, &headers)?;
    post(state, slug, headers, body).await
}

pub async fn upgrade_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    upgrade(state, slug, headers, ws).await
}

pub async fn upgrade_by_host(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let slug = slug_from_host(&state, &headers)?;
    upgrade(state, slug, headers, ws).await
}

async fn post(
    state: AppState,
    slug: String,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let route = db::resolve_project_route(&state.pool, &slug).await?;
    authorize_bearer(&state, &route, &headers)?;
    let origin = project_origin(&state, &route, &headers);
    let response = process_bytes(state, route, origin, &body).await;
    match response {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(payload) => {
            let mut response = payload.to_string().into_response();
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            Ok(response)
        }
    }
}

async fn upgrade(
    state: AppState,
    slug: String,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let route = db::resolve_project_route(&state.pool, &slug).await?;
    authorize_websocket(&state, &route, &headers)?;
    let origin = project_origin(&state, &route, &headers);
    Ok(ws
        .protocols([JSON_RPC_PROTOCOL])
        .max_message_size(MAX_RPC_MESSAGE_BYTES)
        .max_frame_size(MAX_RPC_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(state, route, origin, socket))
        .into_response())
}

fn slug_from_host(state: &AppState, headers: &HeaderMap) -> Result<String, ApiError> {
    let host = header_host(headers).ok_or(ApiError::NotFound)?;
    let suffix = format!(".{}", state.config.project_domain);
    host.to_ascii_lowercase()
        .strip_suffix(&suffix)
        .filter(|value| !value.is_empty() && !value.contains('.'))
        .map(str::to_string)
        .ok_or(ApiError::NotFound)
}

fn header_host(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(':').next())
}

fn authorize_bearer(
    state: &AppState,
    route: &ProjectRoute,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let key = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    authorize_key(state, route, key)
}

fn authorize_websocket(
    state: &AppState,
    route: &ProjectRoute,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let protocols = headers
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let has_json_rpc = protocols
        .split(',')
        .map(str::trim)
        .any(|protocol| protocol == JSON_RPC_PROTOCOL);
    let key = protocols
        .split(',')
        .map(str::trim)
        .find_map(|protocol| protocol.strip_prefix(TOKEN_PROTOCOL_PREFIX))
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::Unauthorized)?;
    if !has_json_rpc {
        return Err(ApiError::Invalid(
            "the jsonrpc WebSocket subprotocol is required".to_string(),
        ));
    }
    authorize_key(state, route, key)
}

fn authorize_key(state: &AppState, route: &ProjectRoute, key: &str) -> Result<(), ApiError> {
    let hash = hash_api_key(key, &state.config.api_key_pepper);
    if is_hash_match(&hash, &route.publishable_key_hash) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn project_origin(state: &AppState, route: &ProjectRoute, headers: &HeaderMap) -> ProjectOrigin {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or(&state.config.project_domain);
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("https"));
    let host_without_port = host.split(':').next().unwrap_or(host);
    let stable_host = host_without_port
        .strip_suffix(&format!(".{}", state.config.project_domain))
        .is_some_and(|value| value == route.slug);
    let http_scheme = if secure { "https" } else { "http" };
    let ws_scheme = if secure { "wss" } else { "ws" };
    let path = if stable_host {
        String::new()
    } else {
        format!("/v1/projects/{}/rpc", route.slug)
    };
    ProjectOrigin {
        http: format!("{http_scheme}://{host}{path}"),
        websocket: format!("{ws_scheme}://{host}{path}"),
    }
}

#[derive(Clone)]
struct ProjectOrigin {
    http: String,
    websocket: String,
}

async fn handle_socket(
    state: AppState,
    route: ProjectRoute,
    origin: ProjectOrigin,
    socket: WebSocket,
) {
    let (mut writer, mut reader) = socket.split();
    let (outbound, mut messages) = mpsc::channel::<Message>(state.config.queue_capacity);
    let writer_task = tokio::spawn(async move {
        while let Some(message) = messages.recv().await {
            if writer.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = reader.next().await {
        match message {
            Ok(Message::Text(body)) => {
                let state = state.clone();
                let route = route.clone();
                let origin = origin.clone();
                let outbound = outbound.clone();
                tokio::spawn(async move {
                    if let Some(response) =
                        process_bytes(state, route, origin, body.as_bytes()).await
                    {
                        let _ = outbound
                            .send(Message::Text(response.to_string().into()))
                            .await;
                    }
                });
            }
            Ok(Message::Binary(_)) => {
                let _ = outbound
                    .send(Message::Text(
                        error_response(Value::Null, RpcError::parse_error())
                            .to_string()
                            .into(),
                    ))
                    .await;
            }
            Ok(Message::Ping(payload)) => {
                let _ = outbound.send(Message::Pong(payload)).await;
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) | Err(_) => break,
        }
    }
    drop(outbound);
    let _ = writer_task.await;
}

async fn process_bytes(
    state: AppState,
    route: ProjectRoute,
    origin: ProjectOrigin,
    body: &[u8],
) -> Option<Value> {
    if body.len() > MAX_RPC_MESSAGE_BYTES {
        return Some(error_response(
            Value::Null,
            RpcError::invalid_request("request is too large"),
        ));
    }
    let payload = match serde_json::from_slice::<Value>(body) {
        Ok(payload) => payload,
        Err(_) => return Some(error_response(Value::Null, RpcError::parse_error())),
    };
    match payload {
        Value::Array(requests) if requests.is_empty() => Some(error_response(
            Value::Null,
            RpcError::invalid_request("batch must not be empty"),
        )),
        Value::Array(requests) => {
            let calls = requests.into_iter().map(|request| {
                process_request(state.clone(), route.clone(), origin.clone(), request)
            });
            let responses = join_all(calls)
                .await
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            (!responses.is_empty()).then_some(Value::Array(responses))
        }
        request => process_request(state, route, origin, request).await,
    }
}

async fn process_request(
    state: AppState,
    route: ProjectRoute,
    origin: ProjectOrigin,
    request: Value,
) -> Option<Value> {
    let parsed = match parse_request(request) {
        Ok(parsed) => parsed,
        Err(error) => return Some(error_response(Value::Null, error)),
    };
    let notification = parsed.id.is_none();
    let result = dispatch(&state, &route, &origin, &parsed.method, parsed.params).await;
    if notification {
        return None;
    }
    let id = parsed.id.unwrap_or(Value::Null);
    Some(match result {
        Ok(result) => json!({ "jsonrpc": JSON_RPC_VERSION, "id": id, "result": result }),
        Err(error) => error_response(id, error),
    })
}

struct RpcRequest {
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

fn parse_request(request: Value) -> Result<RpcRequest, RpcError> {
    let Value::Object(mut request) = request else {
        return Err(RpcError::invalid_request("request must be an object"));
    };
    if request.remove("jsonrpc") != Some(Value::String(JSON_RPC_VERSION.to_string())) {
        return Err(RpcError::invalid_request("jsonrpc must be 2.0"));
    }
    let method = request
        .remove("method")
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RpcError::invalid_request("method must be a non-empty string"))?;
    let id = request.remove("id");
    if id
        .as_ref()
        .is_some_and(|value| !value.is_null() && !value.is_string() && !value.is_number())
    {
        return Err(RpcError::invalid_request(
            "id must be a string, number, or null",
        ));
    }
    let params = request.remove("params");
    Ok(RpcRequest { id, method, params })
}

async fn dispatch(
    state: &AppState,
    project: &ProjectRoute,
    origin: &ProjectOrigin,
    method: &str,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    match method {
        "rpc.discover" => {
            require_empty_params(params)?;
            Ok(project_discovery(project, origin))
        }
        "agent.list" => {
            require_empty_params(params)?;
            let routes = db::list_project_agent_routes(&state.pool, project.id)
                .await
                .map_err(internal_error)?;
            Ok(json!({
                "agents": routes.into_iter().map(agent_descriptor).collect::<Vec<_>>()
            }))
        }
        "agent.invoke" => {
            let params = parse_invoke_params(params)?;
            invoke_agent(state, project, params).await
        }
        _ => Err(RpcError::method_not_found()),
    }
}

fn require_empty_params(params: Option<Value>) -> Result<(), RpcError> {
    match params {
        None => Ok(()),
        Some(Value::Object(params)) if params.is_empty() => Ok(()),
        Some(Value::Array(params)) if params.is_empty() => Ok(()),
        _ => Err(RpcError::invalid_params("this method takes no parameters")),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentInvokeParams {
    agent: Option<String>,
    message: Option<String>,
    input: Option<Value>,
    context: Option<Value>,
    metadata: Option<Value>,
    timeout_ms: Option<u64>,
}

fn parse_invoke_params(params: Option<Value>) -> Result<AgentInvokeParams, RpcError> {
    let Some(Value::Object(params)) = params else {
        return Err(RpcError::invalid_params(
            "agent.invoke requires named parameters",
        ));
    };
    serde_json::from_value(Value::Object(params))
        .map_err(|_| RpcError::invalid_params("invalid agent.invoke parameters"))
}

async fn invoke_agent(
    state: &AppState,
    project: &ProjectRoute,
    params: AgentInvokeParams,
) -> Result<Value, RpcError> {
    if params
        .message
        .as_deref()
        .is_none_or(|message| message.trim().is_empty())
        && params.input.is_none()
    {
        return Err(RpcError::invalid_params(
            "message or input must be provided",
        ));
    }
    let routes = db::list_project_agent_routes(&state.pool, project.id)
        .await
        .map_err(internal_error)?;
    let route = select_agent(routes, params.agent.as_deref())?;
    let timeout_ms = params
        .timeout_ms
        .unwrap_or(state.config.request_timeout.as_millis() as u64);
    if !(500..=120_000).contains(&timeout_ms) {
        return Err(RpcError::invalid_params(
            "timeoutMs must be between 500 and 120000",
        ));
    }
    let timeout =
        Duration::from_millis(timeout_ms.min(state.config.request_timeout.as_millis() as u64));
    let request = json!({
        "message": params.message,
        "input": params.input,
        "context": params.context,
        "metadata": params.metadata,
    });
    if serde_json::to_vec(&request)
        .map_err(|_| RpcError::internal())?
        .len()
        > MAX_RPC_MESSAGE_BYTES
    {
        return Err(RpcError::invalid_params("request is too large"));
    }

    let request_id = Uuid::new_v4();
    let gateway_session_id = state.relay.session_for(&route.gateway_id).await;
    db::create_project_trace(
        &state.pool,
        request_id,
        project.id,
        gateway_session_id,
        &request,
    )
    .await
    .map_err(internal_error)?;
    let started_at = Instant::now();
    let endpoint_route = route.endpoint_route(timeout.as_millis() as i32);
    match state
        .relay
        .invoke(&endpoint_route, request_id, request, timeout)
        .await
    {
        Ok(output) => {
            complete_project_trace(
                state,
                request_id,
                "completed",
                started_at,
                Some(&output),
                None,
            )
            .await;
            Ok(output)
        }
        Err(error) => {
            let (status, rpc_error) = map_relay_error(error);
            complete_project_trace(
                state,
                request_id,
                status,
                started_at,
                None,
                Some(&rpc_error.message),
            )
            .await;
            Err(rpc_error)
        }
    }
}

fn select_agent(
    routes: Vec<ProjectAgentRoute>,
    selector: Option<&str>,
) -> Result<ProjectAgentRoute, RpcError> {
    match selector
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
    {
        Some(selector) => {
            let matches = routes
                .into_iter()
                .filter(|route| {
                    route.profile_slug == selector
                        || route.agent_id == selector
                        || route.binding_id.to_string() == selector
                })
                .collect::<Vec<_>>();
            match matches.len() {
                0 => Err(RpcError::invalid_params(
                    "agent is not bound to this project",
                )),
                1 => Ok(matches.into_iter().next().expect("one route")),
                _ => Err(RpcError::invalid_params(
                    "agent selector is ambiguous; use a profile slug or binding id",
                )),
            }
        }
        None if routes.len() == 1 => Ok(routes.into_iter().next().expect("one route")),
        None if routes.is_empty() => Err(RpcError::invalid_params("project has no agent bindings")),
        None => Err(RpcError::invalid_params(
            "agent is required when a project has multiple bindings",
        )),
    }
}

fn agent_descriptor(route: ProjectAgentRoute) -> Value {
    json!({
        "id": route.profile_slug,
        "name": route.profile_name,
        "agentId": route.agent_id,
        "bindingId": route.binding_id,
    })
}

async fn complete_project_trace(
    state: &AppState,
    request_id: Uuid,
    status: &str,
    started_at: Instant,
    response: Option<&Value>,
    error: Option<&str>,
) {
    if let Err(persist_error) = db::complete_trace(
        &state.pool,
        request_id,
        status,
        db::elapsed_millis(started_at),
        response,
        error,
    )
    .await
    {
        warn!(error = %persist_error, %request_id, "could not complete project trace");
    }
}

fn map_relay_error(error: RelayCallError) -> (&'static str, RpcError) {
    match error {
        RelayCallError::AgentGatewayUnavailable => (
            "unavailable",
            RpcError::server(-32001, "agent gateway is unavailable"),
        ),
        RelayCallError::Backpressure => (
            "rejected",
            RpcError::server(-32002, "agent gateway is busy"),
        ),
        RelayCallError::Timeout => (
            "timed_out",
            RpcError::server(-32003, "agent request timed out"),
        ),
        RelayCallError::AgentGateway(message) => {
            ("failed", RpcError::server(-32004, public_message(&message)))
        }
    }
}

fn public_message(message: &str) -> String {
    let message = message
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect::<String>();
    if message.is_empty() {
        "agent request failed".to_string()
    } else {
        message
    }
}

fn internal_error(error: ApiError) -> RpcError {
    warn!(error = %error, "project RPC request failed");
    RpcError::internal()
}

fn project_discovery(project: &ProjectRoute, origin: &ProjectOrigin) -> Value {
    json!({
        "project": {
            "id": project.id,
            "slug": project.slug,
            "gatewayId": project.gateway_id
        },
        "protocol": {
            "name": "vifu.project",
            "version": "0.1",
            "methods": ["rpc.discover", "agent.list", "agent.invoke"]
        },
        "transports": {
            "http": origin.http,
            "websocket": origin.websocket,
            "jsonrpc": JSON_RPC_VERSION,
            "websocketProtocol": JSON_RPC_PROTOCOL
        },
        "capabilities": ["agent.list", "agent.invoke"]
    })
}

#[derive(Debug)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcError {
    fn parse_error() -> Self {
        Self::server(-32700, "Parse error")
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self::server(-32600, message)
    }

    fn method_not_found() -> Self {
        Self::server(-32601, "Method not found")
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::server(-32602, message)
    }

    fn internal() -> Self {
        Self::server(-32603, "Internal error")
    }

    fn server(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn error_response(id: Value, error: RpcError) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": id,
        "error": { "code": error.code, "message": error.message }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        parse_request, project_discovery, select_agent, ProjectAgentRoute, ProjectOrigin,
        ProjectRoute,
    };

    #[test]
    fn parses_named_json_rpc_requests() {
        let request = parse_request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "agent.invoke",
            "params": { "agent": "guide", "message": "Hello" }
        }))
        .unwrap();
        assert_eq!(request.method, "agent.invoke");
        assert_eq!(request.id, Some(json!(1)));
    }

    #[test]
    fn rejects_non_json_rpc_requests() {
        assert!(parse_request(json!({ "method": "agent.list" })).is_err());
        assert!(parse_request(json!([])).is_err());
    }

    #[test]
    fn discovery_uses_vifu_protocol_shape_instead_of_openrpc() {
        let project = ProjectRoute {
            id: Uuid::new_v4(),
            slug: "demo".to_string(),
            gateway_id: "openclaw-local".to_string(),
            publishable_key_hash: Vec::new(),
        };
        let origin = ProjectOrigin {
            http: "http://demo.localhost:6790".to_string(),
            websocket: "ws://demo.localhost:6790".to_string(),
        };
        let discovery = project_discovery(&project, &origin);

        assert!(discovery.get("openrpc").is_none());
        assert_eq!(discovery["project"]["slug"], "demo");
        assert_eq!(discovery["project"]["gatewayId"], "openclaw-local");
        assert_eq!(discovery["protocol"]["name"], "vifu.project");
        assert_eq!(discovery["protocol"]["version"], "0.1");
        assert_eq!(
            discovery["protocol"]["methods"],
            json!(["rpc.discover", "agent.list", "agent.invoke"])
        );
        assert_eq!(discovery["transports"]["jsonrpc"], "2.0");
        assert_eq!(discovery["transports"]["websocketProtocol"], "jsonrpc");
        assert_eq!(
            discovery["capabilities"],
            json!(["agent.list", "agent.invoke"])
        );
    }

    #[test]
    fn requires_an_agent_selector_for_multiple_bindings() {
        assert!(select_agent(Vec::<ProjectAgentRoute>::new(), None).is_err());
    }
}
