use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("authentication required")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    Invalid(String),
    #[error("resource not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("model is required")]
    ModelRequired,
    #[error("the requested agent is not available to this project key")]
    AgentAccessDenied,
    #[error("the API key does not permit this endpoint")]
    EndpointAccessDenied,
    #[error("the agent gateway credential was revoked")]
    AgentGatewayCredentialRevoked,
    #[error("agent gateway is not available")]
    AgentGatewayUnavailable,
    #[error("agent gateway is busy")]
    Backpressure,
    #[error("agent request timed out")]
    Timeout,
    #[error("the project runtime has not been published")]
    RuntimeNotPublished,
    #[error("the project runtime is temporarily unavailable")]
    RuntimeExtensionUnavailable,
    #[error("deployment authority is temporarily unavailable")]
    DeploymentAuthorityUnavailable,
    #[error("{0}")]
    AgentGateway(String),
    #[error("{0}")]
    Provider(String),
    #[error("database request failed")]
    Database(#[from] sqlx::Error),
    #[error("migration failed")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("internal server error")]
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            Self::Invalid(_) => (StatusCode::BAD_REQUEST, "INVALID_REQUEST"),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            Self::ModelRequired => (StatusCode::BAD_REQUEST, "model_required"),
            Self::AgentAccessDenied => (StatusCode::FORBIDDEN, "agent_access_denied"),
            Self::EndpointAccessDenied => (StatusCode::FORBIDDEN, "endpoint_access_denied"),
            Self::AgentGatewayCredentialRevoked => {
                (StatusCode::CONFLICT, "gateway_credential_revoked")
            }
            Self::AgentGatewayUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, "AGENT_GATEWAY_UNAVAILABLE")
            }
            Self::Backpressure => (StatusCode::TOO_MANY_REQUESTS, "BACKPRESSURE"),
            Self::Timeout => (StatusCode::GATEWAY_TIMEOUT, "REQUEST_TIMEOUT"),
            Self::RuntimeNotPublished => (StatusCode::CONFLICT, "project_runtime_not_published"),
            Self::RuntimeExtensionUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_extension_unavailable",
            ),
            Self::DeploymentAuthorityUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "deployment_authority_unavailable",
            ),
            Self::AgentGateway(_) => (StatusCode::BAD_GATEWAY, "AGENT_GATEWAY_ERROR"),
            Self::Provider(_) => (StatusCode::BAD_GATEWAY, "PROVIDER_ERROR"),
            Self::Database(_) | Self::Migration(_) | Self::Internal => {
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR")
            }
        };
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            "Internal server error".to_string()
        } else {
            self.to_string()
        };
        let error = json!({ "code": code, "message": message });
        (status, Json(json!({ "error": error }))).into_response()
    }
}

pub fn map_database_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(database) = &error {
        if database.is_unique_violation() {
            return ApiError::Conflict("resource already exists".to_string());
        }
        if database.is_foreign_key_violation() {
            return ApiError::Invalid("referenced resource does not exist".to_string());
        }
    }
    ApiError::Database(error)
}
