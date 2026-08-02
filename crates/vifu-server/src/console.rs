use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HOST, ORIGIN};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::time::Duration;

use crate::config::DeploymentMode;
use crate::AppState;

include!(concat!(env!("OUT_DIR"), "/console_assets.rs"));

pub async fn serve_console_asset(State(state): State<AppState>, uri: Uri) -> Response {
    if !local_console_enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(asset_path) = asset_path_for_uri(&uri) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(asset) = asset_for_path(asset_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    asset_response(asset)
}

pub async fn proxy_runtime_request(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    if !local_console_enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin_request(&headers, state.config.addr) {
        return json_error(
            StatusCode::FORBIDDEN,
            "embedded console requests must be same-origin",
        );
    }
    let Some(runtime_path) = runtime_path_for_uri(&uri) else {
        return json_error(StatusCode::NOT_FOUND, "runtime API route was not found");
    };
    if !valid_runtime_path(runtime_path) {
        return json_error(StatusCode::BAD_REQUEST, "runtime API path is invalid");
    }

    let mut target = format!("http://{}/v1/{runtime_path}", state.config.addr);
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }

    let client = reqwest::Client::new();
    let request_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client
        .request(request_method, target)
        .timeout(Duration::from_secs(8))
        .header(ACCEPT.as_str(), "application/json")
        .header(
            AUTHORIZATION_HEADER,
            format!("Vifu {}", state.config.admin_key),
        );
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

fn local_console_enabled(state: &AppState) -> bool {
    state.config.deployment_mode == DeploymentMode::Local && state.config.addr.ip().is_loopback()
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

fn same_origin_request(headers: &HeaderMap, expected_addr: std::net::SocketAddr) -> bool {
    let fetch_site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if fetch_site.eq_ignore_ascii_case("cross-site") {
        return false;
    }

    let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let expected_authority = expected_addr.to_string();
    if !host.eq_ignore_ascii_case(&expected_authority) {
        return false;
    }
    headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|origin| origin.eq_ignore_ascii_case(&format!("http://{expected_authority}")))
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

    use axum::http::{HeaderMap, HeaderValue, Uri};

    use super::{asset_path_for_uri, same_origin_request, valid_runtime_path};

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
