use axum::http::{header::CONTENT_RANGE, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;
use vifu_game_runtime::ValidationIssue;

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
    #[error("byte range is not satisfiable for a {0}-byte resource")]
    RangeNotSatisfiable(u64),
    #[error("{0}")]
    Conflict(String),
    #[error("game source failed validation")]
    Validation(Vec<ValidationIssue>),
    #[error("model is required")]
    ModelRequired,
    #[error("the requested locale is not supported by this game")]
    LocaleNotSupported,
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
        if let Self::RangeNotSatisfiable(size) = &self {
            let mut response = (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Json(json!({
                    "error": {
                        "code": "RANGE_NOT_SATISFIABLE",
                        "message": format!("byte range is not satisfiable for a {size}-byte resource")
                    }
                })),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(CONTENT_RANGE, value);
            }
            return response;
        }
        let (status, code) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            Self::Invalid(_) => (StatusCode::BAD_REQUEST, "INVALID_REQUEST"),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            Self::RangeNotSatisfiable(_) => unreachable!("range errors return above"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            Self::Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED"),
            Self::ModelRequired => (StatusCode::BAD_REQUEST, "model_required"),
            Self::LocaleNotSupported => (StatusCode::BAD_REQUEST, "locale_not_supported"),
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
        let mut error = json!({ "code": code, "message": message });
        if let Self::Validation(issues) = &self {
            error["issues"] = json!(issues);
        }
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
