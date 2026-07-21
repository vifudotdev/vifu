use std::io::SeekFrom;
use std::path::{Component, Path as FilePath, PathBuf};

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::require_admin;
use crate::db as runtime_db;
use crate::error::ApiError;
use crate::AppState;

use super::api::authorize_game_project;
use super::db::{self, NewGameAssetVersion};
use super::models::{ApproveGameAssetVersion, GameAssetVersion};

const MAX_ASSET_BYTES: u64 = 30 * 1024 * 1024;

pub async fn list_asset_versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, asset_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let versions = db::list_game_asset_versions(&state.pool, project.project.id, asset_id).await?;
    Ok(Json(json!({"versions": versions})))
}

pub async fn upload_asset_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, asset_id)): Path<(String, Uuid)>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let asset = db::get_game_asset(&state.pool, project.project.id, asset_id).await?;
    let root = asset_storage_root(&state);
    let temporary_root = root.join("tmp");
    tokio::fs::create_dir_all(&temporary_root)
        .await
        .map_err(|_| ApiError::Internal)?;

    let mut uploaded = None;
    let mut metadata = json!({});
    let mut provenance = json!({});
    let mut rights_status = "unreviewed".to_string();
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::Invalid(format!("multipart body is invalid: {error}")))?
    {
        let field_name = field.name().unwrap_or_default().to_string();
        match field_name.as_str() {
            "file" => {
                if uploaded.is_some() {
                    return Err(ApiError::Invalid(
                        "only one file may be uploaded per asset version".to_string(),
                    ));
                }
                let mime_type = field
                    .content_type()
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                validate_asset_mime(&asset.kind, &mime_type)?;
                let temporary_path = temporary_root.join(Uuid::new_v4().to_string());
                let mut file = File::create(&temporary_path)
                    .await
                    .map_err(|_| ApiError::Internal)?;
                let mut hash = Sha256::new();
                let mut size_bytes = 0_u64;
                while let Some(chunk) = field.chunk().await.map_err(|error| {
                    ApiError::Invalid(format!("asset file could not be read: {error}"))
                })? {
                    size_bytes = size_bytes.saturating_add(chunk.len() as u64);
                    if size_bytes > MAX_ASSET_BYTES {
                        let _ = tokio::fs::remove_file(&temporary_path).await;
                        return Err(ApiError::Invalid(format!(
                            "asset file must not exceed {} MiB",
                            MAX_ASSET_BYTES / 1024 / 1024
                        )));
                    }
                    hash.update(&chunk);
                    file.write_all(&chunk)
                        .await
                        .map_err(|_| ApiError::Internal)?;
                }
                file.flush().await.map_err(|_| ApiError::Internal)?;
                let digest = format!("{:x}", hash.finalize());
                uploaded = Some((temporary_path, mime_type, size_bytes, digest));
            }
            "metadata" => metadata = parse_object_field(field, "metadata").await?,
            "provenance" => provenance = parse_object_field(field, "provenance").await?,
            "rightsStatus" => {
                rights_status = bounded_text_field(field, "rightsStatus").await?;
                validate_rights_status(&rights_status)?;
            }
            _ => {
                return Err(ApiError::Invalid(format!(
                    "unknown multipart field `{field_name}`"
                )));
            }
        }
    }

    let (temporary_path, mime_type, size_bytes, digest) =
        uploaded.ok_or_else(|| ApiError::Invalid("file is required".to_string()))?;
    let content_hash = format!("sha256:{digest}");
    if let Some(version) = db::find_game_asset_version_by_hash(
        &state.pool,
        project.project.id,
        asset_id,
        &content_hash,
    )
    .await?
    {
        let _ = tokio::fs::remove_file(temporary_path).await;
        return Ok((StatusCode::OK, Json(json!({"version": version}))));
    }

    let storage_key = format!("sha256/{digest}");
    let final_path = root.join(&storage_key);
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| ApiError::Internal)?;
    }
    if tokio::fs::try_exists(&final_path)
        .await
        .map_err(|_| ApiError::Internal)?
    {
        tokio::fs::remove_file(&temporary_path)
            .await
            .map_err(|_| ApiError::Internal)?;
    } else {
        tokio::fs::rename(&temporary_path, &final_path)
            .await
            .map_err(|_| ApiError::Internal)?;
    }
    let size_bytes = i64::try_from(size_bytes).map_err(|_| ApiError::Internal)?;
    let version = db::create_game_asset_version(
        &state.pool,
        NewGameAssetVersion {
            project_id: project.project.id,
            asset_id,
            content_hash: &content_hash,
            mime_type: &mime_type,
            size_bytes,
            storage_key: &storage_key,
            metadata: &metadata,
            provenance: &provenance,
            rights_status: &rights_status,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"version": version}))))
}

pub async fn approve_asset_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, asset_id, version_id)): Path<(String, Uuid, Uuid)>,
    Json(input): Json<ApproveGameAssetVersion>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    if !matches!(input.status.as_str(), "approved" | "rejected") {
        return Err(ApiError::Invalid(
            "status must be approved or rejected".to_string(),
        ));
    }
    let version = db::approve_game_asset_version(
        &state.pool,
        project.project.id,
        asset_id,
        version_id,
        &input.status,
    )
    .await?;
    Ok(Json(json!({"version": version})))
}

pub async fn serve_runtime_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, version_id)): Path<(String, Uuid)>,
) -> Result<Response, ApiError> {
    let project = authorize_game_project(&state, &headers, &project_slug).await?;
    let version =
        db::active_presentation_asset(&state.pool, project.project.id, version_id).await?;
    serve_asset_file(&state, &headers, version).await
}

pub async fn serve_authoring_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, asset_id, version_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &project_slug).await?;
    let version = db::get_game_asset_version(&state.pool, project.project.id, version_id).await?;
    if version.asset_id != asset_id {
        return Err(ApiError::NotFound);
    }
    serve_asset_file(&state, &headers, version).await
}

async fn serve_asset_file(
    state: &AppState,
    headers: &HeaderMap,
    version: GameAssetVersion,
) -> Result<Response, ApiError> {
    let storage_key = safe_storage_key(&version.storage_key)?;
    let path = asset_storage_root(state).join(storage_key);
    let mut file = File::open(path).await.map_err(|_| ApiError::NotFound)?;
    let file_size = file.metadata().await.map_err(|_| ApiError::NotFound)?.len();
    if i64::try_from(file_size).ok() != Some(version.size_bytes) {
        return Err(ApiError::Internal);
    }
    let range = headers
        .get(RANGE)
        .map(|value| value.to_str().map_err(|_| invalid_range(file_size)))
        .transpose()?
        .map(|value| parse_byte_range(value, file_size))
        .transpose()?;
    let (status, start, length) = match range {
        Some((start, end)) => {
            file.seek(SeekFrom::Start(start))
                .await
                .map_err(|_| ApiError::Internal)?;
            (StatusCode::PARTIAL_CONTENT, start, end - start + 1)
        }
        None => (StatusCode::OK, 0, file_size),
    };
    let stream = ReaderStream::new(file.take(length));
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    let response_headers = response.headers_mut();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&version.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response_headers.insert(
        ETAG,
        HeaderValue::from_str(&format!("\"{}\"", version.content_hash))
            .map_err(|_| ApiError::Internal)?,
    );
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).map_err(|_| ApiError::Internal)?,
    );
    if status == StatusCode::PARTIAL_CONTENT {
        response_headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{file_size}", start + length - 1))
                .map_err(|_| ApiError::Internal)?,
        );
    }
    Ok(response.into_response())
}

async fn parse_object_field(
    field: axum::extract::multipart::Field<'_>,
    name: &str,
) -> Result<Value, ApiError> {
    let text = bounded_text_field(field, name).await?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| ApiError::Invalid(format!("{name} must be valid JSON")))?;
    if !value.is_object() {
        return Err(ApiError::Invalid(format!("{name} must be a JSON object")));
    }
    Ok(value)
}

async fn bounded_text_field(
    field: axum::extract::multipart::Field<'_>,
    name: &str,
) -> Result<String, ApiError> {
    let text = field
        .text()
        .await
        .map_err(|error| ApiError::Invalid(format!("{name} could not be read: {error}")))?;
    if text.len() > 64 * 1024 {
        return Err(ApiError::Invalid(format!("{name} must not exceed 64 KiB")));
    }
    Ok(text)
}

fn validate_asset_mime(kind: &str, mime_type: &str) -> Result<(), ApiError> {
    let accepted = match kind {
        "image" => mime_type.starts_with("image/"),
        "audio" => mime_type.starts_with("audio/"),
        "video" => mime_type.starts_with("video/"),
        "font" => mime_type.starts_with("font/") || mime_type.contains("font"),
        "subtitle" => matches!(
            mime_type,
            "text/plain" | "text/vtt" | "application/x-subrip" | "application/json"
        ),
        _ => false,
    };
    if accepted {
        Ok(())
    } else {
        Err(ApiError::Invalid(format!(
            "content type `{mime_type}` is not valid for an asset of kind `{kind}`"
        )))
    }
}

fn validate_rights_status(value: &str) -> Result<(), ApiError> {
    if matches!(value, "unreviewed" | "owned" | "licensed" | "public_domain") {
        Ok(())
    } else {
        Err(ApiError::Invalid(
            "rightsStatus must be unreviewed, owned, licensed, or public_domain".to_string(),
        ))
    }
}

fn asset_storage_root(state: &AppState) -> PathBuf {
    state.config.provider_home_dir.join("assets")
}

fn safe_storage_key(value: &str) -> Result<PathBuf, ApiError> {
    let path = FilePath::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ApiError::Internal);
    }
    Ok(path.to_path_buf())
}

fn parse_byte_range(value: &str, file_size: u64) -> Result<(u64, u64), ApiError> {
    let range = value
        .strip_prefix("bytes=")
        .ok_or_else(|| invalid_range(file_size))?;
    if range.contains(',') || file_size == 0 {
        return Err(invalid_range(file_size));
    }
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| invalid_range(file_size))?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| invalid_range(file_size))?;
        if suffix == 0 {
            return Err(invalid_range(file_size));
        }
        let length = suffix.min(file_size);
        return Ok((file_size - length, file_size - 1));
    }
    let start = start.parse::<u64>().map_err(|_| invalid_range(file_size))?;
    let end = if end.is_empty() {
        file_size - 1
    } else {
        end.parse::<u64>()
            .map_err(|_| invalid_range(file_size))?
            .min(file_size - 1)
    };
    if start >= file_size || start > end {
        return Err(invalid_range(file_size));
    }
    Ok((start, end))
}

fn invalid_range(file_size: u64) -> ApiError {
    ApiError::RangeNotSatisfiable(file_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_suffix_ranges() {
        assert_eq!(parse_byte_range("bytes=2-5", 10).unwrap(), (2, 5));
        assert_eq!(parse_byte_range("bytes=7-", 10).unwrap(), (7, 9));
        assert_eq!(parse_byte_range("bytes=-3", 10).unwrap(), (7, 9));
        assert!(parse_byte_range("bytes=12-", 10).is_err());
        assert!(parse_byte_range("bytes=1-2,4-5", 10).is_err());
    }

    #[test]
    fn invalid_ranges_use_standard_http_status_and_content_range() {
        let response = invalid_range(10).into_response();
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers().get(CONTENT_RANGE).unwrap(), "bytes */10");
    }

    #[test]
    fn rejects_unsafe_storage_keys() {
        assert!(safe_storage_key("sha256/abc").is_ok());
        assert!(safe_storage_key("../private").is_err());
        assert!(safe_storage_key("/tmp/private").is_err());
    }
}
