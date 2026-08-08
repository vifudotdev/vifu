use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{
    ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, REFERER,
    SET_COOKIE, USER_AGENT,
};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::time::Duration;

use crate::AppState;

include!(concat!(env!("OUT_DIR"), "/console_assets.rs"));

pub async fn serve_console_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if !embedded_console_enabled(&state) {
        return proxy_dashboard(state, Method::GET, headers, uri, Bytes::new()).await;
    }
    let Some(asset_path) = asset_path_for_uri(&uri) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(asset) = asset_for_path(asset_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    asset_response(asset)
}

pub async fn proxy_dashboard_request(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    proxy_dashboard(state, method, headers, uri, body).await
}

async fn proxy_dashboard(
    state: AppState,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    let Some(address) = state.config.dashboard_addr.as_deref() else {
        return json_error(StatusCode::NOT_FOUND, "resource not found");
    };
    if uri.path() == "/v1" || uri.path().starts_with("/v1/") {
        return json_error(StatusCode::NOT_FOUND, "resource not found");
    }
    let target = dashboard_target(address, &uri);
    let request_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let Ok(client) = dashboard_client() else {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "dashboard proxy could not be initialized",
        );
    };
    let mut request = client
        .request(request_method, target)
        .timeout(Duration::from_secs(30));
    for name in [
        ACCEPT,
        ACCEPT_LANGUAGE,
        CONTENT_TYPE,
        COOKIE,
        HOST,
        ORIGIN,
        REFERER,
        USER_AGENT,
    ] {
        if let Some(value) = headers.get(&name) {
            request = request.header(name.as_str(), value.as_bytes());
        }
    }
    if let Some(host) = headers.get(HOST) {
        request = request.header("x-forwarded-host", host.as_bytes());
    }
    let forwarded_proto = state
        .config
        .server_url
        .as_deref()
        .and_then(|url| reqwest::Url::parse(url).ok())
        .map_or("http", |url| {
            if url.scheme() == "https" {
                "https"
            } else {
                "http"
            }
        });
    request = request.header("x-forwarded-proto", forwarded_proto);
    if !body.is_empty() {
        request = request.body(body.to_vec());
    }
    match request.send().await {
        Ok(response) => dashboard_response(response).await,
        Err(error) => json_error(
            if error.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            },
            "dashboard is temporarily unavailable",
        ),
    }
}

fn dashboard_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

fn dashboard_target(address: &str, uri: &Uri) -> String {
    format!(
        "http://{address}{}",
        uri.path_and_query().map_or("/", |value| value.as_str())
    )
}

async fn dashboard_response(response: reqwest::Response) -> Response {
    const MAX_DASHBOARD_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DASHBOARD_RESPONSE_BYTES)
    {
        return json_error(StatusCode::BAD_GATEWAY, "dashboard response is too large");
    }
    let mut builder = Response::builder().status(status);
    for name in [
        CONTENT_TYPE,
        CACHE_CONTROL,
        SET_COOKIE,
        axum::http::header::LOCATION,
        axum::http::header::ETAG,
        axum::http::header::VARY,
    ] {
        for value in response.headers().get_all(name.as_str()) {
            builder = builder.header(name.clone(), value.as_bytes());
        }
    }
    let bytes = match response.bytes().await {
        Ok(bytes) if bytes.len() as u64 <= MAX_DASHBOARD_RESPONSE_BYTES => bytes,
        Ok(_) => return json_error(StatusCode::BAD_GATEWAY, "dashboard response is too large"),
        Err(_) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                "dashboard response could not be read",
            )
        }
    };
    builder
        .body(Body::from(bytes))
        .expect("dashboard proxy response should be buildable")
}

pub async fn proxy_runtime_request(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    if !embedded_console_enabled(&state) {
        return proxy_dashboard(state, method, headers, uri, body).await;
    }
    if !same_origin_server_request(&headers, &uri, &state) {
        return json_error(
            StatusCode::FORBIDDEN,
            "embedded console requests must be same-origin",
        );
    }
    if !local_console_peer(peer) {
        return json_error(
            StatusCode::FORBIDDEN,
            "embedded console administration is available only on the Server host",
        );
    }
    let Some(runtime_path) = runtime_path_for_uri(&uri) else {
        return json_error(StatusCode::NOT_FOUND, "runtime API route was not found");
    };
    if !valid_runtime_path(runtime_path) {
        return json_error(StatusCode::BAD_REQUEST, "runtime API path is invalid");
    }

    let server_url = state
        .config
        .server_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", state.config.addr));
    let mut target = format!("{}/v1/{runtime_path}", server_url.trim_end_matches('/'));
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }

    let client = match embedded_console_client(&state) {
        Ok(client) => client,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "embedded console proxy could not be initialized",
            );
        }
    };
    let request_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client
        .request(request_method, target)
        .header(ACCEPT.as_str(), "application/json")
        .header(
            AUTHORIZATION_HEADER,
            format!("Vifu {}", state.config.admin_key),
        );
    if let Some(timeout) = runtime_proxy_timeout(&method, runtime_path, Duration::from_secs(8)) {
        request = request.timeout(timeout);
    }
    if let Some(content_type) = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        request = request.header(CONTENT_TYPE.as_str(), content_type);
    }
    if !body.is_empty() {
        request = request.body(body.to_vec());
    }

    match request.send().await {
        Ok(response) => runtime_response(response).await,
        Err(error) => json_error(
            if error.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            },
            &format!("embedded console proxy failed: {error}"),
        ),
    }
}

fn runtime_proxy_timeout(
    method: &Method,
    runtime_path: &str,
    timeout: Duration,
) -> Option<Duration> {
    let server_owns_deadline = *method == Method::POST && runtime_path == "chat/completions";
    (!server_owns_deadline).then_some(timeout)
}

fn embedded_console_enabled(state: &AppState) -> bool {
    state.config.dashboard_addr.is_none()
}

fn local_console_peer(peer: std::net::SocketAddr) -> bool {
    if peer.ip().is_loopback() {
        return true;
    }
    let origin = match peer {
        std::net::SocketAddr::V4(peer) => format!("http://{}:{}", peer.ip(), peer.port()),
        std::net::SocketAddr::V6(peer) => format!("http://[{}]:{}", peer.ip(), peer.port()),
    };
    vifu_gateway::config::local_component_socket_addr(&origin)
        .is_ok_and(|address| address.is_some())
}

fn embedded_console_client(state: &AppState) -> Result<reqwest::Client, String> {
    let mut client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    if let Some(certificate_der) = state
        .server_endpoint
        .as_deref()
        .map(crate::ServerEndpointIdentity::certificate_der)
        .transpose()?
        .flatten()
    {
        let certificate = reqwest::Certificate::from_der(&certificate_der)
            .map_err(|error| format!("embedded Console certificate is invalid: {error}"))?;
        client = client.add_root_certificate(certificate);
    }
    client
        .build()
        .map_err(|error| format!("embedded Console client could not be initialized: {error}"))
}

fn asset_for_path(path: &str) -> Option<&'static ConsoleAsset> {
    CONSOLE_ASSETS.iter().find(|asset| asset.path == path)
}

fn asset_path_for_uri(uri: &Uri) -> Option<&str> {
    let path = uri.path();
    let requested = path.strip_prefix('/')?.trim_end_matches('/');
    if requested.is_empty() {
        return Some("index.html");
    }
    if !valid_asset_path(requested) {
        return None;
    }
    if asset_for_path(requested).is_some() {
        return Some(requested);
    }
    if requested.contains('.') {
        return None;
    }
    let first_segment = requested.split('/').next()?;
    (first_segment == "project").then_some("index.html")
}

fn runtime_path_for_uri(uri: &Uri) -> Option<&str> {
    let path = uri.path().strip_prefix("/api/runtime/")?;
    (!path.is_empty()).then_some(path)
}

fn valid_asset_path(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn valid_runtime_path(path: &str) -> bool {
    valid_asset_path(path)
}

#[cfg(test)]
fn same_origin_request(headers: &HeaderMap, expected_addr: std::net::SocketAddr) -> bool {
    same_origin_request_for(
        headers,
        None,
        &expected_addr.to_string(),
        &format!("http://{expected_addr}"),
    )
}

fn same_origin_server_request(headers: &HeaderMap, uri: &Uri, state: &AppState) -> bool {
    let fallback = format!("http://{}", state.config.addr);
    let server_url = state.config.server_url.as_deref().unwrap_or(&fallback);
    let Ok(server_url) = reqwest::Url::parse(server_url) else {
        return false;
    };
    let Some(host) = server_url.host_str() else {
        return false;
    };
    let authority = match (host.parse::<std::net::Ipv6Addr>(), server_url.port()) {
        (Ok(_), Some(port)) => format!("[{host}]:{port}"),
        (Ok(_), None) => format!("[{host}]"),
        (Err(_), Some(port)) => format!("{host}:{port}"),
        (Err(_), None) => host.to_string(),
    };
    let origin = format!("{}://{authority}", server_url.scheme());
    same_origin_request_for(
        headers,
        uri.authority().map(|value| value.as_str()),
        &authority,
        &origin,
    )
}

fn same_origin_request_for(
    headers: &HeaderMap,
    request_authority: Option<&str>,
    expected_authority: &str,
    expected_origin: &str,
) -> bool {
    let fetch_site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if fetch_site.eq_ignore_ascii_case("cross-site") {
        return false;
    }

    let Some(host) = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .or(request_authority)
    else {
        return false;
    };
    if !host.eq_ignore_ascii_case(expected_authority) {
        return false;
    }
    headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|origin| origin.eq_ignore_ascii_case(expected_origin))
}

fn asset_response(asset: &'static ConsoleAsset) -> Response {
    let mut response = Body::from(Bytes::from_static(asset.bytes)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        asset
            .content_type
            .parse()
            .expect("generated asset content types are valid"),
    );
    headers.insert(
        CACHE_CONTROL,
        if CONSOLE_ASSETS_AVAILABLE && asset.path != "index.html" {
            "public, max-age=31536000, immutable"
        } else {
            "no-store"
        }
        .parse()
        .expect("cache-control values are valid"),
    );
    response
}

async fn runtime_response(response: reqwest::Response) -> Response {
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = response
        .headers()
        .get(CONTENT_TYPE.as_str())
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                &format!("embedded console proxy could not read runtime response: {error}"),
            );
        }
    };
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(CONTENT_TYPE, content_type);
    }
    if let Some(retry_after) = retry_after {
        builder = builder.header("retry-after", retry_after);
    }
    builder
        .body(Body::from(bytes))
        .expect("runtime proxy response should be buildable")
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Body::from(json!({ "error": { "message": message } }).to_string()),
    )
        .into_response()
}

const AUTHORIZATION_HEADER: &str = "authorization";

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use axum::body::Bytes;
    use axum::extract::{ConnectInfo, State};
    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
    use axum::routing::{get, post};
    use axum::Router;
    use sqlx::postgres::PgPoolOptions;
    use tokio::net::TcpListener;

    use super::{
        asset_path_for_uri, dashboard_client, dashboard_target, embedded_console_enabled,
        proxy_runtime_request, runtime_proxy_timeout, same_origin_request,
        same_origin_server_request, valid_runtime_path,
    };
    use crate::config::{Config, DeploymentMode};

    #[test]
    fn console_proxy_only_leaves_invocation_deadlines_to_the_runtime() {
        assert_eq!(
            runtime_proxy_timeout(&Method::GET, "status", Duration::from_secs(8)),
            Some(Duration::from_secs(8)),
        );
        assert_eq!(
            runtime_proxy_timeout(&Method::POST, "projects", Duration::from_secs(8)),
            Some(Duration::from_secs(8)),
        );
        assert_eq!(
            runtime_proxy_timeout(&Method::POST, "chat/completions", Duration::from_secs(8),),
            None,
        );
    }

    #[tokio::test]
    async fn local_console_uses_the_server_listener_on_lan_addresses() {
        let mut config = Config::from_env().unwrap();
        config.deployment_mode = DeploymentMode::Local;
        config.addr = "192.0.2.20:6790".parse().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();

        assert!(embedded_console_enabled(&crate::state(config, pool)));
    }

    #[tokio::test]
    async fn lan_console_accepts_its_public_https_authority() {
        let mut config = Config::from_env().unwrap();
        config.addr = "0.0.0.0:6790".parse().unwrap();
        config.server_url = Some("https://192.168.50.246:6790".to_string());
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let state = crate::state(config, pool);
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("192.168.50.246:6790"));
        headers.insert(
            "origin",
            HeaderValue::from_static("https://192.168.50.246:6790"),
        );

        assert!(same_origin_server_request(
            &headers,
            &"/api/runtime/status".parse().unwrap(),
            &state
        ));

        headers.remove("host");
        assert!(same_origin_server_request(
            &headers,
            &"https://192.168.50.246:6790/api/runtime/status"
                .parse()
                .unwrap(),
            &state
        ));
    }

    #[tokio::test]
    async fn dashboard_proxy_client_leaves_redirects_for_the_browser() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/login",
                    get(|| async {
                        (
                            StatusCode::SEE_OTHER,
                            [("location", "/login?auth_error=invalid")],
                        )
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let response = dashboard_client()
            .unwrap()
            .get(format!("http://{address}/login"))
            .send()
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                response
                    .headers()
                    .get("location")
                    .unwrap()
                    .to_str()
                    .unwrap(),
            ),
            (StatusCode::SEE_OTHER, "/login?auth_error=invalid")
        );
        server.abort();
    }

    #[tokio::test]
    async fn self_hosted_runtime_requests_reach_the_dashboard_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/runtime/project/demo/api-keys",
                    post(|| async { (StatusCode::CREATED, "proxied") }),
                ),
            )
            .await
            .unwrap();
        });

        let mut config = Config::from_env().unwrap();
        config.deployment_mode = DeploymentMode::SelfHosted;
        config.apply_dashboard_addr(address.to_string()).unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let response = proxy_runtime_request(
            State(crate::state(config, pool)),
            ConnectInfo("127.0.0.1:12345".parse().unwrap()),
            Method::POST,
            HeaderMap::new(),
            "/api/runtime/project/demo/api-keys".parse().unwrap(),
            Bytes::from_static(b"{}"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        server.abort();
    }

    #[test]
    fn dashboard_proxy_keeps_the_single_server_path_and_query() {
        let uri: Uri = "/pair?request=pairing-id".parse().unwrap();

        assert_eq!(
            dashboard_target("dashboard:6791", &uri),
            "http://dashboard:6791/pair?request=pairing-id"
        );
    }

    #[test]
    fn console_history_routes_serve_index() {
        let uri: Uri = "/".parse().unwrap();
        assert_eq!(asset_path_for_uri(&uri), Some("index.html"));

        let uri: Uri = "/project/demo/agents".parse().unwrap();
        assert_eq!(asset_path_for_uri(&uri), Some("index.html"));

        let uri: Uri = "/project/demo/agents/".parse().unwrap();
        assert_eq!(asset_path_for_uri(&uri), Some("index.html"));
    }

    #[test]
    fn console_asset_paths_block_parent_segments() {
        let uri: Uri = "/assets/../main.js".parse().unwrap();
        assert_eq!(asset_path_for_uri(&uri), None);
        assert!(!valid_runtime_path("../status"));
    }

    #[test]
    fn console_path_is_not_reserved_for_embedded_ui() {
        let uri: Uri = "/console".parse().unwrap();
        assert_eq!(asset_path_for_uri(&uri), None);
    }

    #[test]
    fn console_proxy_requires_same_origin_when_origin_is_present() {
        let expected: SocketAddr = "127.0.0.1:6790".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("127.0.0.1:6790"));
        headers.insert("origin", HeaderValue::from_static("http://evil.test"));
        assert!(!same_origin_request(&headers, expected));

        headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:6790"));
        assert!(same_origin_request(&headers, expected));
    }

    #[test]
    fn console_proxy_rejects_dns_rebinding_hosts_even_without_an_origin() {
        let expected: SocketAddr = "127.0.0.1:6790".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("attacker.test:6790"));
        headers.insert(
            "origin",
            HeaderValue::from_static("http://attacker.test:6790"),
        );
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(!same_origin_request(&headers, expected));

        headers.remove("origin");
        assert!(!same_origin_request(&headers, expected));
    }

    #[test]
    fn console_proxy_allows_the_configured_loopback_host_without_an_origin() {
        let expected: SocketAddr = "127.0.0.1:6790".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("127.0.0.1:6790"));
        assert!(same_origin_request(&headers, expected));
    }
}
