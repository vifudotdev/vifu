pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod models;
mod openclaw_device;
pub mod relay;
pub mod runtime_extensions;
pub mod websocket;

use std::future::Future;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderName, Method};
use axum::routing::{get, patch, post, put};
use axum::Router;
use config::Config;
use error::ApiError;
use relay::RelayHub;
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
    pub auth: auth::ApplicationAuth,
    pub pool: db::Storage,
    pub relay: RelayHub,
}

pub async fn connect(config: Config) -> Result<AppState, ApiError> {
    let pool = db::connect(&config.database_url, config.database_max_connections)
        .await
        .map_err(|error| diagnose_startup_error("connect", error))?;
    db::migrate(&pool)
        .await
        .map_err(|error| diagnose_startup_error("migrate", error))?;
    db::mark_agent_gateway_sessions_disconnected(&pool)
        .await
        .map_err(|error| diagnose_startup_error("mark gateway sessions disconnected", error))?;
    Ok(state_with_storage(config, pool))
}

fn diagnose_startup_error(stage: &str, error: ApiError) -> ApiError {
    if std::env::var_os("VIFU_SERVER_DIAGNOSTICS").is_some() {
        eprintln!("vifu server startup failed during {stage}: {error:?}");
    }
    error
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
    info!(%addr, "vifu server listening");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|error| format!("http server failed: {error}"))
}

pub fn state(config: Config, pool: PgPool) -> AppState {
    state_with_storage(config, db::Storage::postgres(pool))
}

pub fn state_with_storage(config: Config, pool: db::Storage) -> AppState {
    let auth = auth::ApplicationAuth::from_config(&config);
    state_with_storage_and_auth(config, pool, auth)
}

pub fn state_with_storage_and_auth(
    config: Config,
    pool: db::Storage,
    auth: auth::ApplicationAuth,
) -> AppState {
    let queue_capacity = config.queue_capacity;
    AppState {
        config: Arc::new(config),
        auth,
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
            "/v1/auth/exchange",
            post(api::exchange_deployment_credential),
        )
        .route("/v1/guest/bootstrap", post(api::bootstrap_guest_project))
        .route("/v1/guest/claim", post(api::claim_guest_project))
        .route("/v1/admin/verify", get(api::verify_admin))
        .route(
            "/v1/admin/project-ownership",
            get(api::list_project_ownership),
        )
        .route(
            "/v1/admin/project-ownership/{project_id}",
            axum::routing::patch(api::assign_project_owner),
        )
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
            "/v1/project/{slug}/deployments",
            get(api::list_project_runtime_deployments).post(api::create_project_runtime_deployment),
        )
        .route(
            "/v1/project/{slug}/deployments/{deployment}",
            patch(api::update_project_runtime_deployment)
                .delete(api::delete_project_runtime_deployment),
        )
        .route(
            "/v1/project/{slug}/deployments/{deployment}/promote",
            post(api::promote_project_runtime_deployment),
        )
        .route(
            "/v1/project/{slug}/deployments/{deployment}/agent-gateway-enrollments",
            post(api::create_runtime_deployment_agent_gateway_enrollment),
        )
        .route(
            "/v1/project/{slug}/deployments/{deployment}/agent-gateways/{gateway_id}",
            post(api::assign_runtime_deployment_agent_gateway)
                .delete(api::unassign_runtime_deployment_agent_gateway),
        )
        .route(
            "/v1/project/{slug}/runtime-releases",
            get(api::list_project_runtime_releases).post(api::publish_project_runtime_release),
        )
        .route(
            "/v1/project/{slug}/runtime-releases/{version}",
            get(api::get_project_runtime_release),
        )
        .route(
            "/v1/project/{slug}/deployments/{deployment}/runtime-releases/{version}/activate",
            post(api::activate_project_runtime_release),
        )
        .route(
            "/v1/runtime-extensions",
            get(runtime_extensions::list_runtime_extensions),
        )
        .route(
            "/v1/runtime-extensions/{extension_id}/projects/{project_id}/invoke",
            post(runtime_extensions::invoke_project_profile_for_extension),
        )
        .route(
            "/v1/project/{slug}/extensions/runtime",
            get(runtime_extensions::get_project_runtime_extension)
                .put(runtime_extensions::set_project_runtime_extension)
                .delete(runtime_extensions::delete_project_runtime_extension),
        )
        .route(
            "/v1/project/{slug}/runtime-channels",
            get(runtime_extensions::list_project_runtime_channels)
                .post(runtime_extensions::create_project_runtime_channel),
        )
        .route(
            "/v1/project/{slug}/runtime-channels/{channel_id}",
            axum::routing::delete(runtime_extensions::delete_project_runtime_channel),
        )
        .route("/v1/provider-adapters", get(api::list_provider_adapters))
        .route("/v1/provider-catalog", get(api::list_provider_catalog))
        .route(
            "/v1/project/{slug}/provider-catalog",
            get(api::list_project_provider_catalog),
        )
        .route(
            "/v1/project/{slug}/providers",
            get(api::list_project_providers).post(api::create_project_provider),
        )
        .route(
            "/v1/project/{slug}/providers/import",
            post(api::import_project_provider),
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
            "/v1/project/{slug}/provider-adapters",
            get(api::list_project_provider_adapters),
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
            "/v1/project/{slug}/profiles/import",
            post(api::import_project_profile),
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
            "/v1/project/{slug}/bindings",
            get(api::list_project_bindings).post(api::create_project_binding),
        )
        .route(
            "/v1/project/{slug}/bindings/{id}",
            get(api::get_project_binding)
                .patch(api::update_project_binding)
                .delete(api::delete_project_binding),
        )
        .route(
            "/v1/project/{slug}/endpoints",
            get(api::list_project_endpoints).post(api::create_project_endpoint),
        )
        .route(
            "/v1/project/{slug}/endpoints/{id}",
            get(api::get_project_endpoint)
                .patch(api::update_project_endpoint)
                .delete(api::delete_project_endpoint),
        )
        .route(
            "/v1/project/{slug}/api-keys",
            get(api::list_project_api_keys).post(api::create_project_api_key),
        )
        .route(
            "/v1/project/{slug}/api-keys/{id}",
            patch(api::update_project_api_key).delete(api::delete_project_api_key),
        )
        .route(
            "/v1/project/{slug}/api-keys/{id}/revoke",
            post(api::revoke_project_api_key),
        )
        .route(
            "/v1/project/{slug}/agent-gateways",
            get(api::list_project_agent_gateways),
        )
        .route(
            "/v1/project/{slug}/agents",
            get(api::list_project_available_agents),
        )
        .route("/v1/project/{slug}/traces", get(api::list_project_traces))
        .route(
            "/v1/project/{slug}/traces/{id}/spans",
            get(api::list_project_trace_spans),
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
        .route(
            "/{project_slug}/v1/embeddings",
            post(api::create_project_embeddings),
        )
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
        .route(
            "/{project_slug}/v1/rpc",
            get(runtime_extensions::connect_project_runtime)
                .post(runtime_extensions::invoke_project_runtime),
        )
        .route(
            "/{project_slug}/v1/runtime/launch",
            post(runtime_extensions::create_runtime_launch_session),
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
            "/v1/agent-gateway-pairings",
            get(api::list_agent_gateway_pairings),
        )
        .route(
            "/v1/agent-gateway-pairings/{id}",
            get(api::get_agent_gateway_pairing),
        )
        .route(
            "/v1/agent-gateway-pairings/{id}/approve",
            post(api::approve_agent_gateway_pairing),
        )
        .route(
            "/v1/agent-gateway-pairings/{id}/reject",
            post(api::reject_agent_gateway_pairing),
        )
        .route(
            "/v1/agent-gateway/runtime-config",
            get(api::get_agent_gateway_runtime_config),
        )
        .route(
            "/v1/agent-gateway/runtime-traces",
            post(api::upload_agent_gateway_runtime_traces),
        )
        .route(
            "/v1/agent-gateway/runtime-releases/bootstrap",
            post(api::bootstrap_agent_gateway_runtime_release),
        )
        .route(
            "/v1/project/{slug}/agent-gateway-enrollments",
            post(api::create_project_agent_gateway_enrollment),
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
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(RequestBodyLimitLayer::new(32 * 1024 * 1024))
        .layer(cors)
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::{app, state, state_with_storage, state_with_storage_and_auth, AppState};
    use crate::auth::{
        AccessTokenAuth, AccessTokenAuthFuture, ApplicationAuth, Identity, Operation,
    };
    use crate::config::Config;
    use crate::db::Storage;
    use crate::error::ApiError;

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
    async fn admin_verification_requires_the_configured_key() {
        let config = Config::from_env().unwrap();
        let admin_key = config.admin_key.clone();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();

        let denied = app(state(config.clone(), pool.clone()))
            .oneshot(
                Request::get("/v1/admin/verify")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let verified = app(state(config, pool))
            .oneshot(
                Request::get("/v1/admin/verify")
                    .header("authorization", format!("Bearer {admin_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verified.status(), StatusCode::OK);
        let body = to_bytes(verified.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["valid"], true);
    }

    #[tokio::test]
    async fn resource_routes_reject_unexchanged_account_access_tokens() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();

        let (state, _) =
            state_with_access_token_auth(config, pool, vec![Operation::ProjectRead]).await;
        let response = app(state)
            .oneshot(
                Request::get("/v1/admin/verify")
                    .header("authorization", "Vifu account-access-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn account_access_tokens_are_exchanged_for_deployment_credentials() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let auth = ApplicationAuth::with_deployment_credential_auth(
            config.admin_key.clone(),
            "dep_01JTESTDEPLOYMENT",
            Arc::new(StaticAccessTokenAuth {
                subject: "user-123".to_string(),
                operations: vec![Operation::ProjectRead, Operation::ProjectWrite],
            }),
        );
        let response = app(state_with_storage_and_auth(
            config,
            Storage::postgres(pool),
            auth,
        ))
        .oneshot(
            Request::post("/v1/auth/exchange")
                .header("authorization", "Bearer account-access-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert!(payload["credential"]
            .as_str()
            .is_some_and(|value| value.starts_with("vifu_dc1.")));
        assert!(payload["expiresAt"].as_u64().is_some());
    }

    #[tokio::test]
    async fn read_only_access_tokens_cannot_mutate_a_deployment() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();

        let (state, credential) =
            state_with_access_token_auth(config, pool, vec![Operation::ProjectRead]).await;
        let response = app(state)
            .oneshot(
                Request::post("/v1/projects")
                    .header("authorization", format!("Vifu {credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"Denied project"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn account_projects_are_created_and_listed_under_the_canonical_user() {
        let path = std::env::temp_dir().join(format!(
            "vifu-account-projects-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let config = Config::from_env().unwrap();
        let (owner_state, owner_credential) = state_with_storage_access_token_auth(
            config.clone(),
            storage.clone(),
            "user-123",
            vec![Operation::ProjectRead, Operation::ProjectWrite],
        )
        .await;
        let owner_app = app(owner_state);

        let created = owner_app
            .clone()
            .oneshot(
                Request::post("/v1/projects")
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"Owned project"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let project_id = payload["project"]["id"].as_str().unwrap();
        assert!(payload["project"].get("ownerUserId").is_none());

        let owner_list = owner_app
            .oneshot(
                Request::get("/v1/projects")
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(owner_list.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["projects"].as_array().unwrap().len(), 1);

        let (other_state, other_credential) = state_with_storage_access_token_auth(
            config,
            storage.clone(),
            "user-456",
            vec![Operation::ProjectRead, Operation::ProjectWrite],
        )
        .await;
        let other_app = app(other_state);
        let other_list = other_app
            .clone()
            .oneshot(
                Request::get("/v1/projects")
                    .header("authorization", format!("Vifu {other_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(other_list.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert!(payload["projects"].as_array().unwrap().is_empty());
        let forbidden = other_app
            .oneshot(
                Request::get(format!("/v1/projects/{project_id}"))
                    .header("authorization", format!("Vifu {other_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(pool) => pool.close().await,
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn account_project_enrollment_registers_one_gateway_once() {
        let path = std::env::temp_dir().join(format!(
            "vifu-gateway-enrollment-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let config = Config::from_env().unwrap();
        let api_key_pepper = config.api_key_pepper.clone();
        let (owner_state, owner_credential) = state_with_storage_access_token_auth(
            config,
            storage.clone(),
            "user-123",
            vec![Operation::ProjectRead, Operation::ProjectWrite],
        )
        .await;
        let owner_app = app(owner_state);
        let created = owner_app
            .clone()
            .oneshot(
                Request::post("/v1/projects")
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"Gateway project"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(created.into_body(), 64 * 1024).await.unwrap();
        let project: Value = serde_json::from_slice(&body).unwrap();
        let slug = project["project"]["slug"].as_str().unwrap();

        let enrollment = owner_app
            .clone()
            .oneshot(
                Request::post(format!("/v1/project/{slug}/agent-gateway-enrollments"))
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(enrollment.status(), StatusCode::CREATED);
        let body = to_bytes(enrollment.into_body(), 64 * 1024).await.unwrap();
        let enrollment: Value = serde_json::from_slice(&body).unwrap();
        let token = enrollment["enrollmentToken"].as_str().unwrap();

        let machine = vifu_gateway::identity::MachineIdentity::generate().unwrap();
        crate::db::upsert_agent_gateway_machine(&storage, &machine.machine_id, &machine.public_key)
            .await
            .unwrap();
        let enrollment_hash = crate::auth::hash_agent_gateway_enrollment(token, &api_key_pepper);
        let assignment = crate::db::consume_agent_gateway_machine_enrollment(
            &storage,
            &enrollment_hash,
            "gateway-account",
        )
        .await
        .unwrap();
        let device_token =
            "vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let device_token_hash =
            crate::auth::hash_agent_gateway_credential(device_token, &api_key_pepper);
        crate::db::create_agent_gateway_authorization(
            &storage,
            crate::db::NewAgentGatewayAuthorization {
                gateway_id: "gateway-account",
                machine_id: &machine.machine_id,
                owner_user_id: Some(&assignment.owner_user_id),
                token_prefix: &device_token.chars().take(20).collect::<String>(),
                token_hash: &device_token_hash,
                token_expires_at: chrono::Utc::now() + chrono::Duration::days(180),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            crate::db::get_project_by_slug(&storage, slug)
                .await
                .unwrap()
                .project
                .gateway_id,
            "gateway-account"
        );

        let runtime_config = owner_app
            .clone()
            .oneshot(
                Request::get("/v1/agent-gateway/runtime-config")
                    .header(
                        "authorization",
                        "Bearer vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(runtime_config.status(), StatusCode::OK);
        let body = to_bytes(runtime_config.into_body(), 64 * 1024)
            .await
            .unwrap();
        let runtime_config: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(runtime_config["gatewayId"], "gateway-account");
        assert_eq!(runtime_config["deployments"].as_array().unwrap().len(), 1);
        let deployment_id = runtime_config["deployments"][0]["deploymentId"]
            .as_str()
            .unwrap();
        let project_id = crate::db::get_project_by_slug(&storage, slug)
            .await
            .unwrap()
            .project
            .id;
        assert!(!crate::db::runtime_deployment_allows_remote_invocation(
            &storage,
            project_id,
            "gateway-account",
        )
        .await
        .unwrap());

        let enabled_remote_invocation = owner_app
            .clone()
            .oneshot(
                Request::patch(format!("/v1/project/{slug}/deployments/development"))
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"remoteInvocationEnabled":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(enabled_remote_invocation.status(), StatusCode::OK);
        assert!(crate::db::runtime_deployment_allows_remote_invocation(
            &storage,
            project_id,
            "gateway-account",
        )
        .await
        .unwrap());

        let release = owner_app
            .clone()
            .oneshot(
                Request::post("/v1/agent-gateway/runtime-releases/bootstrap")
                    .header(
                        "authorization",
                        "Bearer vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{
                            "deploymentId":"{deployment_id}",
                            "manifest":{{
                                "schemaVersion":1,
                                "projectId":"{slug}",
                                "providers":[{{
                                    "id":"local-model",
                                    "providerType":"native",
                                    "capabilities":["chat"]
                                }}],
                                "agents":[{{
                                    "id":"guide",
                                    "name":"Guide",
                                    "provider":"local-model",
                                    "capabilities":["chat"]
                                }}],
                                "endpoints":[{{
                                    "name":"guide",
                                    "agent":"guide",
                                    "capability":"chat",
                                    "timeoutMs":30000
                                }}],
                                "metadata":{{}}
                            }}
                        }}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(release.status(), StatusCode::CREATED);

        let runtime_config = owner_app
            .clone()
            .oneshot(
                Request::get("/v1/agent-gateway/runtime-config")
                    .header(
                        "authorization",
                        "Bearer vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(runtime_config.into_body(), 64 * 1024)
            .await
            .unwrap();
        let runtime_config: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(runtime_config["deployments"][0]["release"]["version"], 1);

        let uploaded = owner_app
            .clone()
            .oneshot(
                Request::post("/v1/agent-gateway/runtime-traces")
                    .header(
                        "authorization",
                        "Bearer vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{
                            "deploymentId":"{deployment_id}",
                            "traces":[{{
                                "id":"trace-embedded-1",
                                "projectId":"{slug}",
                                "invocationId":"invoke-embedded-1",
                                "endpoint":"guide",
                                "agent":"guide",
                                "provider":"local-model",
                                "capability":"chat",
                                "status":"completed",
                                "durationMs":12,
                                "createdAtMs":1
                            }}]
                        }}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::OK);
        assert_eq!(
            crate::db::list_traces(
                &storage,
                None,
                Some(
                    crate::db::get_project_by_slug(&storage, slug)
                        .await
                        .unwrap()
                        .project
                        .id,
                ),
                10,
            )
            .await
            .unwrap()
            .len(),
            1
        );
        crate::db::open_agent_gateway_session(
            &storage,
            "gateway-account",
            None,
            &serde_json::json!([{
                "id": "guide",
                "name": "Guide",
                "metadata": {
                    "providerKey": "openclaw-local",
                    "providerType": "openclaw"
                }
            }]),
            &serde_json::json!({}),
        )
        .await
        .unwrap();

        let candidates = owner_app
            .clone()
            .oneshot(
                Request::get(format!("/v1/project/{slug}/agent-candidates"))
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(candidates.status(), StatusCode::OK);
        let body = to_bytes(candidates.into_body(), 64 * 1024).await.unwrap();
        let candidates: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(candidates["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(candidates["candidates"][0]["providerKey"], "openclaw-local");

        let imported = owner_app
            .clone()
            .oneshot(
                Request::post(format!("/v1/project/{slug}/agents/import"))
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{
                            "gatewayId":"gateway-account",
                            "agentId":"guide",
                            "providerKey":"openclaw-local"
                        }"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(imported.status(), StatusCode::CREATED);

        let retried = crate::db::consume_agent_gateway_machine_enrollment(
            &storage,
            &enrollment_hash,
            "gateway-account",
        )
        .await
        .unwrap();
        assert_eq!(retried.project_id, project_id);

        let staging = owner_app
            .clone()
            .oneshot(
                Request::post(format!("/v1/project/{slug}/deployments"))
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"staging"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(staging.status(), StatusCode::CREATED);
        let staging_enrollment = owner_app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/v1/project/{slug}/deployments/staging/agent-gateway-enrollments"
                ))
                .header("authorization", format!("Vifu {owner_credential}"))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(staging_enrollment.into_body(), 64 * 1024)
            .await
            .unwrap();
        let staging_enrollment: Value = serde_json::from_slice(&body).unwrap();
        let staging_token = staging_enrollment["enrollmentToken"].as_str().unwrap();
        let staging_hash =
            crate::auth::hash_agent_gateway_enrollment(staging_token, &api_key_pepper);
        crate::db::consume_agent_gateway_machine_enrollment(
            &storage,
            &staging_hash,
            "gateway-account",
        )
        .await
        .unwrap();
        let runtime_config = owner_app
            .clone()
            .oneshot(
                Request::get("/v1/agent-gateway/runtime-config")
                    .header(
                        "authorization",
                        "Bearer vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(runtime_config.into_body(), 64 * 1024)
            .await
            .unwrap();
        let runtime_config: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(runtime_config["deployments"].as_array().unwrap().len(), 1);
        assert_eq!(runtime_config["deployments"][0]["deployment"], "staging");
        assert!(!crate::db::runtime_deployment_allows_remote_invocation(
            &storage,
            project_id,
            "gateway-account",
        )
        .await
        .unwrap());

        assert!(matches!(
            crate::db::consume_agent_gateway_machine_enrollment(
                &storage,
                &enrollment_hash,
                "gateway-replay",
            )
            .await,
            Err(ApiError::Unauthorized)
        ));

        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(pool) => pool.close().await,
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn guest_gateway_bootstrap_is_idempotent_and_claimable() {
        let path = std::env::temp_dir().join(format!(
            "vifu-guest-project-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let mut config = Config::from_env().unwrap();
        config
            .apply_guest_bootstrap(true, std::time::Duration::from_secs(7 * 24 * 60 * 60), 10)
            .unwrap();
        let api_key_pepper = config.api_key_pepper.clone();
        let (owner_state, owner_credential) = state_with_storage_access_token_auth(
            config,
            storage.clone(),
            "user-guest-owner",
            vec![Operation::ProjectRead, Operation::ProjectWrite],
        )
        .await;
        let guest_app = app(owner_state);
        let gateway_credential =
            "vifu_gw_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let machine = vifu_gateway::identity::MachineIdentity::generate().unwrap();
        crate::db::upsert_agent_gateway_machine(&storage, &machine.machine_id, &machine.public_key)
            .await
            .unwrap();
        let credential_hash =
            crate::auth::hash_agent_gateway_credential(gateway_credential, &api_key_pepper);
        crate::db::create_agent_gateway_authorization(
            &storage,
            crate::db::NewAgentGatewayAuthorization {
                gateway_id: "gateway-guest",
                machine_id: &machine.machine_id,
                owner_user_id: None,
                token_prefix: &gateway_credential.chars().take(20).collect::<String>(),
                token_hash: &credential_hash,
                token_expires_at: chrono::Utc::now() + chrono::Duration::days(180),
            },
        )
        .await
        .unwrap();

        let created = guest_app
            .clone()
            .oneshot(
                Request::post("/v1/guest/bootstrap")
                    .header("authorization", format!("Bearer {gateway_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), 64 * 1024).await.unwrap();
        let created: Value = serde_json::from_slice(&body).unwrap();
        let project_id = created["project"]["id"].as_str().unwrap();
        let project_slug = created["project"]["slug"].as_str().unwrap();
        let api_key = created["apiKey"].as_str().unwrap();
        let claim_token = created["claimToken"].as_str().unwrap();
        assert!(api_key.starts_with("vifu_pk_"));
        assert!(claim_token.starts_with("vifu_gc_"));
        assert_eq!(created["project"].as_object().unwrap().len(), 2);
        assert_eq!(created["deployment"].as_object().unwrap().len(), 2);
        assert_eq!(created["deployment"]["name"], "development");

        let repeated = guest_app
            .clone()
            .oneshot(
                Request::post("/v1/guest/bootstrap")
                    .header("authorization", format!("Bearer {gateway_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(repeated.status(), StatusCode::OK);
        let body = to_bytes(repeated.into_body(), 64 * 1024).await.unwrap();
        let repeated: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(repeated["project"]["id"], project_id);
        assert_eq!(repeated["apiKey"], api_key);
        assert_eq!(repeated["claimToken"], claim_token);

        let runtime_config = guest_app
            .clone()
            .oneshot(
                Request::get("/v1/agent-gateway/runtime-config")
                    .header(
                        "authorization",
                        "Bearer vifu_gw_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(runtime_config.status(), StatusCode::OK);
        let body = to_bytes(runtime_config.into_body(), 64 * 1024)
            .await
            .unwrap();
        let runtime_config: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            runtime_config["deployments"][0]["projectSlug"],
            project_slug
        );
        assert_eq!(
            runtime_config["deployments"][0]["policies"]["remoteInvocation"],
            true
        );

        let claimed = guest_app
            .clone()
            .oneshot(
                Request::post("/v1/guest/claim")
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"claimToken":"{claim_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(claimed.status(), StatusCode::OK);

        let projects = guest_app
            .clone()
            .oneshot(
                Request::get("/v1/projects")
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(projects.into_body(), 64 * 1024).await.unwrap();
        let projects: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects["projects"].as_array().unwrap().len(), 1);
        assert_eq!(projects["projects"][0]["id"], project_id);

        let replayed = guest_app
            .oneshot(
                Request::post("/v1/guest/claim")
                    .header("authorization", format!("Vifu {owner_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"claimToken":"{claim_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(pool) => pool.close().await,
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn deployment_admin_can_assign_a_legacy_project_to_an_account() {
        let path = std::env::temp_dir().join(format!(
            "vifu-project-ownership-migration-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let project_id = uuid::Uuid::new_v4();
        crate::db::create_project(
            &storage,
            crate::db::NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "legacy-project",
                name: "Legacy project",
                description: None,
                gateway_id: "project-legacy-project",
                binding_ids: &[],
            },
        )
        .await
        .unwrap();

        let config = Config::from_env().unwrap();
        let admin_key = config.admin_key.clone();
        let (account_state, account_credential) = state_with_storage_access_token_auth(
            config.clone(),
            storage.clone(),
            "user-123",
            vec![Operation::ProjectRead, Operation::ProjectWrite],
        )
        .await;
        let account_app = app(account_state);
        let denied = account_app
            .clone()
            .oneshot(
                Request::patch(format!("/v1/admin/project-ownership/{project_id}"))
                    .header("authorization", format!("Vifu {account_credential}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"ownerUserId":"user-123"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let admin_app = app(state_with_storage(config, storage.clone()));
        let ownership = admin_app
            .clone()
            .oneshot(
                Request::get("/v1/admin/project-ownership")
                    .header("authorization", format!("Bearer {admin_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ownership.status(), StatusCode::OK);
        let body = to_bytes(ownership.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["projects"][0]["ownerUserId"], Value::Null);

        let assigned = admin_app
            .oneshot(
                Request::patch(format!("/v1/admin/project-ownership/{project_id}"))
                    .header("authorization", format!("Bearer {admin_key}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"ownerUserId":"user-123"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(assigned.status(), StatusCode::OK);

        let visible = account_app
            .oneshot(
                Request::get("/v1/projects")
                    .header("authorization", format!("Vifu {account_credential}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(visible.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["projects"].as_array().unwrap().len(), 1);
        assert_eq!(payload["projects"][0]["slug"], "legacy-project");
        assert!(payload["projects"][0].get("ownerUserId").is_none());

        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(pool) => pool.close().await,
        }
        std::fs::remove_file(path).unwrap();
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
        assert_eq!(revoke.status(), StatusCode::UNAUTHORIZED);

        let update = app(state(config.clone(), pool.clone()))
            .oneshot(
                Request::patch(format!("/v1/api-keys/{key_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentScope":{"mode":"all"}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::UNAUTHORIZED);

        let delete = app(state(config, pool))
            .oneshot(
                Request::delete(format!("/v1/api-keys/{key_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::UNAUTHORIZED);
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

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn project_embeddings_route_is_registered() {
        let config = Config::from_env().unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://vifu@127.0.0.1:1/vifu")
            .unwrap();
        let response = app(state(config, pool))
            .oneshot(
                Request::post("/test/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"guide","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
            Request::post("/v1/project/test/providers/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "providerKey":"demo-openai",
                        "name":"Demo provider",
                        "providerType":"openai-compatible",
                        "baseUrl":"http://127.0.0.1:9999/v1"
                    }"#,
                ))
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
            Request::post("/v1/project/test/profiles/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "archiveId":"profile-1",
                        "name":"Guide",
                        "activeVersionId":"version-1",
                        "versions":[{"archiveId":"version-1"}]
                    }"#,
                ))
                .unwrap(),
        ];

        for request in requests {
            let response = app(state(config.clone(), pool.clone()))
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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

    async fn state_with_access_token_auth(
        config: Config,
        pool: sqlx::PgPool,
        operations: Vec<Operation>,
    ) -> (AppState, String) {
        state_with_storage_access_token_auth(
            config,
            Storage::postgres(pool),
            "user-123",
            operations,
        )
        .await
    }

    async fn state_with_storage_access_token_auth(
        config: Config,
        storage: Storage,
        subject: &str,
        operations: Vec<Operation>,
    ) -> (AppState, String) {
        let auth = ApplicationAuth::with_deployment_credential_auth(
            config.admin_key.clone(),
            "dep_01JTESTDEPLOYMENT",
            Arc::new(StaticAccessTokenAuth {
                subject: subject.to_string(),
                operations,
            }),
        );
        let credential = auth
            .exchange_access_token("account-access-token")
            .await
            .unwrap()
            .credential;
        (
            state_with_storage_and_auth(config, storage, auth),
            credential,
        )
    }

    struct StaticAccessTokenAuth {
        subject: String,
        operations: Vec<Operation>,
    }

    impl AccessTokenAuth for StaticAccessTokenAuth {
        fn is_authorized<'a>(
            &'a self,
            access_token: &'a str,
            _operation: Operation,
        ) -> AccessTokenAuthFuture<'a> {
            Box::pin(async move {
                if access_token != "account-access-token" {
                    return Err(ApiError::Forbidden);
                }
                Ok(Identity::ActingUser {
                    subject: self.subject.clone(),
                    issuer: "test-authority".to_string(),
                    operations: self.operations.clone(),
                })
            })
        }
    }
}
