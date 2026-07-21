pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod game;
pub mod models;
mod openclaw_device;
pub mod relay;
pub mod websocket;

use std::future::Future;
use std::sync::Arc;

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderName, Method};
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use config::Config;
use error::ApiError;
use relay::RelayHub;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

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

pub async fn serve<F>(config: Config, shutdown: F) -> Result<(), String>
where
    F: Future<Output = ()> + Send + 'static,
{
    let addr = config.addr;
    let state = connect(config).await.map_err(|error| error.to_string())?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|error| format!("could not bind {addr}: {error}"))?;
    game::spawn_effect_worker(state.clone());
    info!(%addr, "vifu server listening");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|error| format!("http server failed: {error}"))
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
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            HeaderName::from_static("last-event-id"),
        ])
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
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
        .route("/v1/projects/{slug}/canvas", get(api::get_project_canvas))
        .route("/v1/project/{slug}/canvas", get(api::get_project_canvas))
        .route("/v1/project/{slug}/game", get(game::api::get_game_overview))
        .route(
            "/v1/project/{slug}/game/source",
            get(game::api::get_game_source).put(game::api::put_game_source),
        )
        .route(
            "/v1/project/{slug}/game/source/import",
            post(game::api::import_game_source),
        )
        .route(
            "/v1/project/{slug}/game/source/export",
            get(game::api::export_game_source),
        )
        .route(
            "/v1/project/{slug}/game/validate",
            post(game::api::validate_game),
        )
        .route(
            "/v1/project/{slug}/game/publish",
            post(game::api::publish_game),
        )
        .route(
            "/v1/project/{slug}/game/releases",
            get(game::api::list_game_releases),
        )
        .route(
            "/v1/project/{slug}/game/releases/{release_id}/activate",
            post(game::api::activate_game_release),
        )
        .route(
            "/v1/project/{slug}/game/resources",
            get(game::management_api::list_resources).post(game::management_api::create_resource),
        )
        .route(
            "/v1/project/{slug}/game/resources/{resource_id}",
            patch(game::management_api::update_resource)
                .delete(game::management_api::delete_resource),
        )
        .route(
            "/v1/project/{slug}/game/assets",
            get(game::management_api::list_assets).post(game::management_api::create_asset),
        )
        .route(
            "/v1/project/{slug}/game/assets/{asset_id}",
            delete(game::management_api::delete_asset),
        )
        .route(
            "/v1/project/{slug}/game/assets/{asset_id}/versions",
            get(game::assets::list_asset_versions).post(game::assets::upload_asset_version),
        )
        .route(
            "/v1/project/{slug}/game/assets/{asset_id}/versions/{version_id}/approve",
            post(game::assets::approve_asset_version),
        )
        .route(
            "/v1/project/{slug}/game/builds",
            post(game::management_api::create_build),
        )
        .route(
            "/v1/project/{slug}/game/builds/{build_id}",
            get(game::management_api::get_build),
        )
        .route(
            "/v1/project/{slug}/game/builds/{build_id}/cancel",
            post(game::management_api::cancel_build),
        )
        .route(
            "/v1/project/{slug}/game/preview",
            post(game::management_api::preview_game),
        )
        .route(
            "/v1/project/{slug}/game/qa",
            get(game::management_api::game_qa),
        )
        .route(
            "/v1/project/{slug}/game/analytics",
            get(game::management_api::game_analytics),
        )
        .route(
            "/v1/project/{slug}/game/sessions",
            get(game::management_api::list_sessions),
        )
        .route(
            "/v1/project/{slug}/game/sessions/{session_id}",
            get(game::management_api::get_session),
        )
        .route(
            "/v1/project/{slug}/game/presentations",
            get(game::management_api::list_presentations)
                .post(game::management_api::publish_presentation),
        )
        .route(
            "/v1/project/{slug}/game/presentations/{presentation_id}/activate",
            post(game::management_api::activate_presentation),
        )
        .route(
            "/v1/game/node-definitions",
            get(game::api::list_node_definitions),
        )
        .route(
            "/v1/projects/{slug}/canvas/nodes",
            post(api::create_canvas_node),
        )
        .route(
            "/v1/project/{slug}/canvas/nodes",
            post(api::create_canvas_node),
        )
        .route(
            "/v1/projects/{slug}/canvas/nodes/{id}",
            axum::routing::patch(api::update_canvas_node).delete(api::delete_canvas_node),
        )
        .route(
            "/v1/project/{slug}/canvas/nodes/{id}",
            axum::routing::patch(api::update_canvas_node).delete(api::delete_canvas_node),
        )
        .route(
            "/v1/projects/{slug}/canvas/edges",
            post(api::create_canvas_edge),
        )
        .route(
            "/v1/project/{slug}/canvas/edges",
            post(api::create_canvas_edge),
        )
        .route(
            "/v1/projects/{slug}/canvas/edges/{id}",
            delete(api::delete_canvas_edge),
        )
        .route(
            "/v1/project/{slug}/canvas/edges/{id}",
            delete(api::delete_canvas_edge),
        )
        .route("/v1/provider-adapters", get(api::list_provider_adapters))
        .route("/v1/provider-catalog", get(api::list_provider_catalog))
        .route(
            "/v1/project/{slug}/providers",
            get(api::list_project_providers).post(api::create_project_provider),
        )
        .route(
            "/v1/project/{slug}/providers/{provider_key}",
            patch(api::update_project_provider).delete(api::delete_project_provider),
        )
        .route(
            "/v1/project/{slug}/providers/{provider_key}/test",
            post(api::test_project_provider),
        )
        .route(
            "/v1/project/{slug}/agent-candidates",
            get(api::list_project_agent_candidates),
        )
        .route(
            "/v1/project/{slug}/agents/import",
            post(api::import_project_agent),
        )
        .route(
            "/v1/project/{slug}/agents/{profile_id}/restore",
            post(api::restore_project_agent),
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
            "/v1/project/{slug}/profiles",
            get(api::list_project_profiles).post(api::create_project_profile),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}",
            get(api::get_project_profile)
                .patch(api::update_project_profile)
                .delete(api::archive_project_profile),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}/versions",
            post(api::create_project_profile_version),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}/source/sync",
            post(api::sync_project_profile_source),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}/versions/{version_id}/activate",
            post(api::activate_project_profile_version),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}/versions/{version_id}/archive",
            post(api::archive_project_profile_version),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}/rollout",
            put(api::set_project_profile_rollout),
        )
        .route(
            "/v1/project/{slug}/profiles/{id}/test",
            post(api::test_project_profile),
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
        .route(
            "/{project_slug}/v1/models",
            get(api::list_project_openai_models),
        )
        .route(
            "/{project_slug}/v1/chat/completions",
            post(api::create_project_chat_completion),
        )
        .route("/{project_slug}/v1/game", get(game::api::get_runtime_game))
        .route(
            "/{project_slug}/v1/game/manifest",
            get(game::api::get_runtime_manifest),
        )
        .route(
            "/{project_slug}/v1/game/presentation",
            get(game::api::get_runtime_presentation),
        )
        .route(
            "/{project_slug}/v1/game/assets/{version_id}",
            get(game::assets::serve_runtime_asset),
        )
        .route(
            "/{project_slug}/v1/game/sessions",
            post(game::api::create_runtime_session),
        )
        .route(
            "/{project_slug}/v1/game/sessions/{session_id}",
            get(game::api::get_runtime_session),
        )
        .route(
            "/{project_slug}/v1/game/sessions/{session_id}/commands",
            post(game::api::submit_runtime_command),
        )
        .route(
            "/{project_slug}/v1/game/sessions/{session_id}/events",
            get(game::api::stream_runtime_events),
        )
        .route("/{project_slug}/v1/game/run", post(game::api::run_game))
        .route("/{project_slug}/v1/agents", get(api::list_project_agents))
        .route(
            "/{project_slug}/v1/audio/speech",
            post(api::create_project_speech),
        )
        .route(
            "/{project_slug}/v1/audio/transcriptions",
            post(api::create_project_transcription),
        )
        .route(
            "/{project_slug}/v1/realtime/sessions",
            post(api::create_realtime_session),
        )
        .route("/{project_slug}/v1/realtime", get(api::connect_realtime))
        .route("/v1/models", get(api::list_openai_models))
        .route("/v1/chat/completions", post(api::create_chat_completion))
        .route(
            "/v1/api-keys",
            get(api::list_api_keys).post(api::create_api_key),
        )
        .route("/v1/api-keys/{id}/revoke", post(api::revoke_api_key))
        .route(
            "/v1/api-keys/{id}",
            patch(api::update_api_key).delete(api::delete_api_key),
        )
        .route("/v1/agents", get(api::list_available_agents))
        .route("/v1/agent-gateways", get(api::list_agent_gateways))
        .route(
            "/v1/agent-gateways/register",
            post(api::register_agent_gateway),
        )
        .route(
            "/v1/agent-gateways/{gateway_id}/revoke",
            post(api::revoke_agent_gateway),
        )
        .route("/v1/traces", get(api::list_traces))
        .route("/v1/traces/{id}/spans", get(api::list_trace_spans))
        .route("/v1/agent-gateway/connect", get(websocket::upgrade))
        .fallback(api::fallback)
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .layer(RequestBodyLimitLayer::new(32 * 1024 * 1024))
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

    #[tokio::test]
    async fn api_key_mutation_routes_require_admin_authority() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let key_id = uuid::Uuid::new_v4();

        let revoke = app(state(config.clone(), pool.clone()))
            .oneshot(
                Request::post(format!("/v1/api-keys/{key_id}/revoke"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::FORBIDDEN);

        let update = app(state(config.clone(), pool.clone()))
            .oneshot(
                Request::patch(format!("/v1/api-keys/{key_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentScope":{"mode":"all"}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::FORBIDDEN);

        let delete = app(state(config, pool))
            .oneshot(
                Request::delete(format!("/v1/api-keys/{key_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn project_profile_delete_route_is_registered() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let profile_id = uuid::Uuid::new_v4();
        let response = app(state(config, pool))
            .oneshot(
                Request::delete(format!("/v1/project/test/profiles/{profile_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn project_agent_and_provider_routes_require_admin_authority() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let requests = [
            Request::get("/v1/provider-catalog")
                .body(Body::empty())
                .unwrap(),
            Request::get("/v1/project/test/providers")
                .body(Body::empty())
                .unwrap(),
            Request::get("/v1/project/test/agent-candidates")
                .body(Body::empty())
                .unwrap(),
            Request::post("/v1/project/test/agents/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"gatewayId":"gateway","agentId":"guide","providerKey":"openclaw"}"#,
                ))
                .unwrap(),
            Request::post(format!(
                "/v1/project/test/agents/{}/restore",
                uuid::Uuid::new_v4()
            ))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        ];

        for request in requests {
            let response = app(state(config.clone(), pool.clone()))
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn legacy_provider_connection_routes_are_not_registered() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let response = app(state(config, pool))
            .oneshot(
                Request::get("/v1/project/test/provider-connections")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
