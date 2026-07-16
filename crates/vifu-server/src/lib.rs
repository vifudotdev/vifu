pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod relay;
pub mod websocket;

use std::sync::Arc;

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderName, Method};
use axum::routing::{delete, get, post};
use axum::Router;
use config::Config;
use error::ApiError;
use relay::RelayHub;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: PgPool,
    pub relay: RelayHub,
}

pub async fn connect(config: Config) -> Result<AppState, ApiError> {
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect(&config.database_url)
        .await?;
    db::migrate(&pool).await?;
    db::mark_agent_gateway_sessions_disconnected(&pool).await?;
    Ok(state(config, pool))
}

pub fn state(config: Config, pool: PgPool) -> AppState {
    let queue_capacity = config.queue_capacity;
    AppState {
        config: Arc::new(config),
        pool,
        relay: RelayHub::new(queue_capacity),
    }
}

pub fn app(state: AppState) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ]);

    Router::new()
        .route("/health", get(api::health))
        .route("/v1/status", get(api::status))
        .route(
            "/v1/projects",
            get(api::list_projects).post(api::create_project),
        )
        .route(
            "/v1/projects/{id}",
            get(api::get_project)
                .patch(api::update_project)
                .delete(api::delete_project),
        )
        .route(
            "/v1/profiles",
            get(api::list_profiles).post(api::create_profile),
        )
        .route(
            "/v1/profiles/{id}",
            get(api::get_profile)
                .patch(api::update_profile)
                .delete(api::delete_profile),
        )
        .route(
            "/v1/bindings",
            get(api::list_bindings).post(api::create_binding),
        )
        .route(
            "/v1/bindings/{id}",
            get(api::get_binding)
                .patch(api::update_binding)
                .delete(api::delete_binding),
        )
        .route(
            "/v1/endpoints",
            get(api::list_endpoints).post(api::create_endpoint),
        )
        .route(
            "/v1/endpoints/{id}",
            get(api::get_endpoint)
                .patch(api::update_endpoint)
                .delete(api::delete_endpoint),
        )
        .route("/v1/endpoints/{id}/invoke", post(api::invoke_endpoint))
        .route(
            "/v1/api-keys",
            get(api::list_api_keys).post(api::create_api_key),
        )
        .route("/v1/api-keys/{id}", delete(api::revoke_api_key))
        .route("/v1/agents", get(api::list_available_agents))
        .route("/v1/agent-gateways", get(api::list_agent_gateways))
        .route("/v1/traces", get(api::list_traces))
        .route("/v1/agent-gateway/connect", get(websocket::upgrade))
        .fallback(api::fallback)
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(cors)
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::{app, state};
    use crate::config::Config;

    #[tokio::test]
    async fn health_is_public_and_does_not_require_database_readiness() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let response = app(state(config, pool))
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["service"], "vifu-server");
    }

    #[tokio::test]
    async fn unknown_routes_use_the_json_error_contract() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let response = app(state(config, pool))
            .oneshot(Request::get("/missing").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
