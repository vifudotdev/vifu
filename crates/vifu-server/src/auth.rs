use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::config::{AccessTokenAuthorityConfig, Config};
use crate::error::ApiError;

pub type AccessTokenAuthFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Identity, ApiError>> + Send + 'a>>;

const DEPLOYMENT_CREDENTIAL_PREFIX: &str = "vifu_dc1";
const DEPLOYMENT_CREDENTIAL_LIFETIME: Duration = Duration::from_secs(5 * 60);
const DEPLOYMENT_CREDENTIAL_CLOCK_SKEW: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    DeploymentRead,
    DeploymentWrite,
    ProjectRead,
    ProjectWrite,
}

impl Operation {
    pub const fn as_scope(self) -> &'static str {
        match self {
            Self::DeploymentRead => "deployment:read",
            Self::DeploymentWrite => "deployment:write",
            Self::ProjectRead => "project:read",
            Self::ProjectWrite => "project:write",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    DeploymentAdmin,
    ActingUser {
        subject: String,
        issuer: String,
        operations: Vec<Operation>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedDeploymentCredential {
    pub credential: String,
    pub expires_at: u64,
}

pub trait AccessTokenAuth: Send + Sync {
    fn is_authorized<'a>(
        &'a self,
        access_token: &'a str,
        operation: Operation,
    ) -> AccessTokenAuthFuture<'a>;
}

#[derive(Debug, Default)]
pub struct NullAccessTokenAuth;

impl AccessTokenAuth for NullAccessTokenAuth {
    fn is_authorized<'a>(
        &'a self,
        _access_token: &'a str,
        _operation: Operation,
    ) -> AccessTokenAuthFuture<'a> {
        Box::pin(async { Err(ApiError::Forbidden) })
    }
}

#[derive(Clone)]
pub struct ApplicationAuth {
    admin_key: Arc<str>,
    access_token_auth: Arc<dyn AccessTokenAuth>,
    deployment_credential_issuer: Option<Arc<DeploymentCredentialIssuer>>,
}

impl ApplicationAuth {
    pub fn from_config(config: &Config) -> Self {
        Self::with_access_token_authority(
            config.admin_key.clone(),
            config.access_token_authority.clone(),
            config.request_timeout,
        )
    }

    pub fn with_access_token_authority(
        admin_key: impl Into<Arc<str>>,
        authority: Option<AccessTokenAuthorityConfig>,
        timeout: std::time::Duration,
    ) -> Self {
        let admin_key = admin_key.into();
        let deployment_credential_issuer = authority.as_ref().map(|authority| {
            Arc::new(DeploymentCredentialIssuer::new(
                admin_key.clone(),
                authority.deployment_id.clone(),
            ))
        });
        let access_token_auth: Arc<dyn AccessTokenAuth> = match authority {
            Some(authority) => Arc::new(HttpAccessTokenAuth::new(authority, timeout)),
            None => Arc::new(NullAccessTokenAuth),
        };
        Self {
            admin_key,
            access_token_auth,
            deployment_credential_issuer,
        }
    }

    pub fn new(
        admin_key: impl Into<Arc<str>>,
        access_token_auth: Arc<dyn AccessTokenAuth>,
    ) -> Self {
        Self {
            admin_key: admin_key.into(),
            access_token_auth,
            deployment_credential_issuer: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_deployment_credential_auth(
        admin_key: impl Into<Arc<str>>,
        deployment_id: impl Into<String>,
        access_token_auth: Arc<dyn AccessTokenAuth>,
    ) -> Self {
        let admin_key = admin_key.into();
        Self {
            deployment_credential_issuer: Some(Arc::new(DeploymentCredentialIssuer::new(
                admin_key.clone(),
                deployment_id.into(),
            ))),
            admin_key,
            access_token_auth,
        }
    }

    pub async fn authorize(
        &self,
        headers: &HeaderMap,
        operation: Operation,
    ) -> Result<Identity, ApiError> {
        let token = deployment_credential(headers).ok_or(ApiError::Unauthorized)?;
        self.authorize_token(token, operation).await
    }

    pub async fn authorize_token(
        &self,
        token: &str,
        operation: Operation,
    ) -> Result<Identity, ApiError> {
        let identity = if is_secret_match(token, &self.admin_key) {
            Identity::DeploymentAdmin
        } else if let Some(issuer) = &self.deployment_credential_issuer {
            issuer.verify(token, SystemTime::now())?
        } else {
            return Err(ApiError::Unauthorized);
        };
        require_operation(&identity, operation)?;
        Ok(identity)
    }

    pub async fn authorize_project(
        &self,
        headers: &HeaderMap,
        operation: Operation,
        owner_user_id: Option<&str>,
    ) -> Result<Identity, ApiError> {
        let token = deployment_credential(headers).ok_or(ApiError::Unauthorized)?;
        let identity = self.authorize_token(token, operation).await?;
        require_project_operation(&identity, operation, owner_user_id)?;
        Ok(identity)
    }

    pub async fn exchange_access_token(
        &self,
        access_token: &str,
    ) -> Result<IssuedDeploymentCredential, ApiError> {
        let issuer = self
            .deployment_credential_issuer
            .as_ref()
            .ok_or(ApiError::Forbidden)?;
        let identity = self
            .access_token_auth
            .is_authorized(access_token, Operation::ProjectRead)
            .await?;
        require_operation(&identity, Operation::ProjectRead)?;
        issuer.issue(&identity, SystemTime::now())
    }
}

#[derive(Debug)]
struct DeploymentCredentialIssuer {
    deployment_id: String,
    signing_key: hmac::Key,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentCredentialPayload {
    version: u8,
    deployment_id: String,
    subject: String,
    issuer: String,
    operations: Vec<String>,
    issued_at: u64,
    expires_at: u64,
}

impl DeploymentCredentialIssuer {
    fn new(admin_key: Arc<str>, deployment_id: String) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"vifu-deployment-credential-signing-key-v1");
        digest.update([0]);
        digest.update(admin_key.as_bytes());
        Self {
            deployment_id,
            signing_key: hmac::Key::new(hmac::HMAC_SHA256, digest.finalize().as_slice()),
        }
    }

    fn issue(
        &self,
        identity: &Identity,
        now: SystemTime,
    ) -> Result<IssuedDeploymentCredential, ApiError> {
        let Identity::ActingUser {
            subject,
            issuer,
            operations,
        } = identity
        else {
            return Err(ApiError::Forbidden);
        };
        let issued_at = unix_seconds(now)?;
        let expires_at = issued_at
            .checked_add(DEPLOYMENT_CREDENTIAL_LIFETIME.as_secs())
            .ok_or(ApiError::Internal)?;
        let payload = DeploymentCredentialPayload {
            version: 1,
            deployment_id: self.deployment_id.clone(),
            subject: subject.clone(),
            issuer: issuer.clone(),
            operations: operations
                .iter()
                .map(|operation| operation.as_scope().to_string())
                .collect(),
            issued_at,
            expires_at,
        };
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).map_err(|_| ApiError::Internal)?);
        let signing_input = format!("{DEPLOYMENT_CREDENTIAL_PREFIX}.{payload}");
        let signature =
            URL_SAFE_NO_PAD.encode(hmac::sign(&self.signing_key, signing_input.as_bytes()));
        Ok(IssuedDeploymentCredential {
            credential: format!("{signing_input}.{signature}"),
            expires_at: expires_at.checked_mul(1000).ok_or(ApiError::Internal)?,
        })
    }

    fn verify(&self, credential: &str, now: SystemTime) -> Result<Identity, ApiError> {
        if credential.len() > 8192 {
            return Err(ApiError::Unauthorized);
        }
        let mut parts = credential.split('.');
        let prefix = parts.next();
        let payload = parts.next();
        let signature = parts.next();
        if prefix != Some(DEPLOYMENT_CREDENTIAL_PREFIX)
            || payload.is_none()
            || signature.is_none()
            || parts.next().is_some()
        {
            return Err(ApiError::Unauthorized);
        }
        let payload = payload.ok_or(ApiError::Unauthorized)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature.ok_or(ApiError::Unauthorized)?)
            .map_err(|_| ApiError::Unauthorized)?;
        let signing_input = format!("{DEPLOYMENT_CREDENTIAL_PREFIX}.{payload}");
        hmac::verify(&self.signing_key, signing_input.as_bytes(), &signature)
            .map_err(|_| ApiError::Unauthorized)?;
        let payload = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| ApiError::Unauthorized)?;
        let payload: DeploymentCredentialPayload =
            serde_json::from_slice(&payload).map_err(|_| ApiError::Unauthorized)?;
        let now = unix_seconds(now)?;
        if payload.version != 1
            || payload.deployment_id != self.deployment_id
            || payload.issued_at > now.saturating_add(DEPLOYMENT_CREDENTIAL_CLOCK_SKEW)
            || payload.expires_at <= now
            || payload.expires_at.saturating_sub(payload.issued_at)
                > DEPLOYMENT_CREDENTIAL_LIFETIME.as_secs()
        {
            return Err(ApiError::Unauthorized);
        }
        let subject = validated_identity_value(payload.subject)?;
        let issuer = validated_identity_value(payload.issuer)?;
        let mut operations = payload
            .operations
            .into_iter()
            .map(|operation| operation_from_scope(&operation))
            .collect::<Result<Vec<_>, _>>()?;
        operations.sort_by_key(|operation| operation.as_scope());
        operations.dedup();
        if operations.is_empty() {
            return Err(ApiError::Unauthorized);
        }
        Ok(Identity::ActingUser {
            subject,
            issuer,
            operations,
        })
    }
}

fn unix_seconds(now: SystemTime) -> Result<u64, ApiError> {
    now.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| ApiError::Internal)
}

fn validated_identity_value(value: String) -> Result<String, ApiError> {
    let value = value.trim().to_string();
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ApiError::Unauthorized);
    }
    Ok(value)
}

fn operation_from_scope(scope: &str) -> Result<Operation, ApiError> {
    match scope {
        "deployment:read" => Ok(Operation::DeploymentRead),
        "deployment:write" => Ok(Operation::DeploymentWrite),
        "project:read" => Ok(Operation::ProjectRead),
        "project:write" => Ok(Operation::ProjectWrite),
        _ => Err(ApiError::Unauthorized),
    }
}

pub fn require_operation(identity: &Identity, operation: Operation) -> Result<(), ApiError> {
    match identity {
        Identity::DeploymentAdmin => Ok(()),
        Identity::ActingUser { operations, .. } => {
            let allowed = operations.contains(&operation)
                || (operation == Operation::DeploymentRead
                    && operations.contains(&Operation::DeploymentWrite))
                || (operation == Operation::ProjectRead
                    && operations.contains(&Operation::ProjectWrite));
            if allowed {
                Ok(())
            } else {
                Err(ApiError::Forbidden)
            }
        }
    }
}

pub fn require_project_operation(
    identity: &Identity,
    operation: Operation,
    owner_user_id: Option<&str>,
) -> Result<(), ApiError> {
    require_operation(identity, operation)?;
    match identity {
        Identity::DeploymentAdmin => Ok(()),
        Identity::ActingUser { subject, .. } if owner_user_id == Some(subject.as_str()) => Ok(()),
        Identity::ActingUser { .. } => Err(ApiError::Forbidden),
    }
}

pub fn require_admin(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, expected) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[derive(Debug)]
struct HttpAccessTokenAuth {
    authority: AccessTokenAuthorityConfig,
    client: reqwest::Client,
    timeout: std::time::Duration,
}

impl HttpAccessTokenAuth {
    fn new(authority: AccessTokenAuthorityConfig, timeout: std::time::Duration) -> Self {
        Self {
            authority,
            client: reqwest::Client::new(),
            timeout,
        }
    }
}

impl AccessTokenAuth for HttpAccessTokenAuth {
    fn is_authorized<'a>(
        &'a self,
        access_token: &'a str,
        operation: Operation,
    ) -> AccessTokenAuthFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.authority.url)
                .bearer_auth(access_token)
                .timeout(self.timeout)
                .json(&AccessTokenAuthorizationRequest {
                    deployment_id: &self.authority.deployment_id,
                    action: operation.as_scope(),
                })
                .send()
                .await
                .map_err(|_| ApiError::DeploymentAuthorityUnavailable)?;
            match response.status() {
                reqwest::StatusCode::UNAUTHORIZED => return Err(ApiError::Unauthorized),
                reqwest::StatusCode::FORBIDDEN => return Err(ApiError::Forbidden),
                status if !status.is_success() => {
                    return Err(ApiError::DeploymentAuthorityUnavailable)
                }
                _ => {}
            }
            let authorization = response
                .json::<AccessTokenAuthorizationResponse>()
                .await
                .map_err(|_| ApiError::DeploymentAuthorityUnavailable)?;
            authorization.identity()
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessTokenAuthorizationRequest<'a> {
    deployment_id: &'a str,
    action: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessTokenAuthorizationResponse {
    is_authorized: bool,
    subject: Option<String>,
    issuer: Option<String>,
    #[serde(default)]
    allowed_operations: Vec<String>,
}

impl AccessTokenAuthorizationResponse {
    fn identity(self) -> Result<Identity, ApiError> {
        if !self.is_authorized {
            return Err(ApiError::Forbidden);
        }
        let subject = self
            .subject
            .map(|value| value.trim().to_string())
            .filter(|value| {
                !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
            })
            .ok_or(ApiError::DeploymentAuthorityUnavailable)?;
        let issuer = self
            .issuer
            .map(|value| value.trim().to_string())
            .filter(|value| {
                !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
            })
            .unwrap_or_else(|| "access-token-authority".to_string());
        let operations = self
            .allowed_operations
            .into_iter()
            .filter_map(|operation| match operation.as_str() {
                "deployment:read" => Some(Operation::DeploymentRead),
                "deployment:write" => Some(Operation::DeploymentWrite),
                "project:read" => Some(Operation::ProjectRead),
                "project:write" => Some(Operation::ProjectWrite),
                _ => None,
            })
            .collect();
        Ok(Identity::ActingUser {
            subject,
            issuer,
            operations,
        })
    }
}

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    authorization_token(headers, "Bearer ")
}

pub fn deployment_credential(headers: &HeaderMap) -> Option<&str> {
    authorization_token(headers, "Vifu ").or_else(|| bearer_token(headers))
}

fn authorization_token<'a>(headers: &'a HeaderMap, prefix: &str) -> Option<&'a str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix(prefix)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn hash_api_key(value: &str, pepper: &str) -> Vec<u8> {
    hash_credential(b"vifu-project-api-key-v1", value, pepper)
}

pub fn hash_agent_gateway_credential(value: &str, pepper: &str) -> Vec<u8> {
    hash_credential(b"vifu-agent-gateway-credential-v1", value, pepper)
}

pub fn hash_agent_gateway_enrollment(value: &str, pepper: &str) -> Vec<u8> {
    hash_credential(b"vifu-agent-gateway-enrollment-v1", value, pepper)
}

fn hash_credential(domain: &[u8], value: &str, pepper: &str) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update([0]);
    digest.update(pepper.as_bytes());
    digest.update([0]);
    digest.update(value.as_bytes());
    digest.finalize().to_vec()
}

pub fn is_secret_match(actual: &str, expected: &str) -> bool {
    constant_time_eq(actual.as_bytes(), expected.as_bytes())
}

pub fn is_hash_match(actual: &[u8], expected: &[u8]) -> bool {
    constant_time_eq(actual, expected)
}

pub fn encrypt_secret_json(value: &str, secret: &str) -> Result<String, ApiError> {
    let key = provider_secret_key(secret)?;
    let rng = SystemRandom::new();
    let mut nonce_bytes = [0_u8; 12];
    rng.fill(&mut nonce_bytes).map_err(|_| ApiError::Internal)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut payload = value.as_bytes().to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut payload)
        .map_err(|_| ApiError::Internal)?;
    let mut encoded = Vec::with_capacity(nonce_bytes.len() + payload.len());
    encoded.extend_from_slice(&nonce_bytes);
    encoded.extend_from_slice(&payload);
    Ok(STANDARD.encode(encoded))
}

pub fn decrypt_secret_json(value: &str, secret: &str) -> Result<String, ApiError> {
    let key = provider_secret_key(secret)?;
    let mut payload = STANDARD
        .decode(value)
        .map_err(|_| ApiError::Invalid("provider secret is invalid".to_string()))?;
    if payload.len() <= 12 {
        return Err(ApiError::Invalid("provider secret is invalid".to_string()));
    }
    let mut nonce_bytes = [0_u8; 12];
    nonce_bytes.copy_from_slice(&payload[..12]);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let plaintext = {
        let ciphertext = &mut payload[12..];
        key.open_in_place(nonce, Aad::empty(), ciphertext)
            .map_err(|_| ApiError::Invalid("provider secret is invalid".to_string()))?
            .to_vec()
    };
    String::from_utf8(plaintext)
        .map_err(|_| ApiError::Invalid("provider secret is invalid".to_string()))
}

fn provider_secret_key(secret: &str) -> Result<LessSafeKey, ApiError> {
    let mut digest = Sha256::new();
    digest.update(b"vifu-provider-secret-key-v1");
    digest.update([0]);
    digest.update(secret.as_bytes());
    let key_bytes = digest.finalize();
    let key =
        UnboundKey::new(&AES_256_GCM, key_bytes.as_slice()).map_err(|_| ApiError::Internal)?;
    Ok(LessSafeKey::new(key))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};

    use crate::config::AccessTokenAuthorityConfig;
    use crate::error::ApiError;

    use super::{
        constant_time_eq, decrypt_secret_json, encrypt_secret_json, hash_api_key,
        require_project_operation, AccessTokenAuth, AccessTokenAuthFuture, ApplicationAuth,
        DeploymentCredentialIssuer, Identity, Operation, DEPLOYMENT_CREDENTIAL_LIFETIME,
    };

    #[test]
    fn compares_secrets() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"other"));
    }

    #[test]
    fn peppers_api_key_hashes() {
        assert_ne!(
            hash_api_key("key", "pepper-a"),
            hash_api_key("key", "pepper-b")
        );
    }

    #[test]
    fn encrypts_provider_secrets() {
        let encrypted = encrypt_secret_json(r#"{"token":"secret"}"#, "provider-key").unwrap();
        assert!(!encrypted.contains("secret"));
        assert_eq!(
            decrypt_secret_json(&encrypted, "provider-key").unwrap(),
            r#"{"token":"secret"}"#
        );
    }

    #[tokio::test]
    async fn authorizes_the_deployment_admin_without_calling_access_token_auth() {
        let auth = ApplicationAuth::new(
            "test-deployment-admin-key",
            Arc::new(RejectingAccessTokenAuth),
        );
        let headers = deployment_headers("test-deployment-admin-key");

        assert_eq!(
            auth.authorize(&headers, Operation::DeploymentWrite)
                .await
                .unwrap(),
            Identity::DeploymentAdmin
        );
    }

    #[tokio::test]
    async fn exchanges_non_admin_credentials_before_local_authorization() {
        let auth = ApplicationAuth::with_deployment_credential_auth(
            "test-deployment-admin-key",
            "dep_01JTESTDEPLOYMENT",
            Arc::new(StaticAccessTokenAuth {
                expected: "account-access-token",
                operations: vec![Operation::ProjectRead],
            }),
        );
        let issued = auth
            .exchange_access_token("account-access-token")
            .await
            .unwrap();
        let headers = deployment_headers(&issued.credential);

        assert!(matches!(
            auth.authorize(&headers, Operation::ProjectRead).await.unwrap(),
            Identity::ActingUser { subject, .. } if subject == "user-123"
        ));
        assert!(matches!(
            auth.authorize(&headers, Operation::ProjectWrite).await,
            Err(ApiError::Forbidden)
        ));
    }

    #[tokio::test]
    async fn self_hosted_auth_rejects_unknown_credentials() {
        let auth = ApplicationAuth::new(
            "test-deployment-admin-key",
            Arc::new(RejectingAccessTokenAuth),
        );

        assert!(matches!(
            auth.authorize(
                &bearer_headers("unknown-credential"),
                Operation::DeploymentRead
            )
            .await,
            Err(ApiError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn exchanges_the_account_token_once_and_reuses_the_deployment_credential_locally() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/authorize",
                    post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                        assert_eq!(
                            headers.get(AUTHORIZATION).unwrap(),
                            "Bearer account-access-token"
                        );
                        assert_eq!(body["deploymentId"], "dep_01JTESTDEPLOYMENT");
                        assert_eq!(body["action"], "project:read");
                        assert!(body.get("runtimeProjectId").is_none());
                        assert!(body.get("projectSlug").is_none());
                        Json(json!({
                            "isAuthorized": true,
                            "subject": "user-123",
                            "issuer": "test-authority",
                            "allowedOperations": ["project:read", "project:write"]
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let auth = ApplicationAuth::with_access_token_authority(
            "test-deployment-admin-key",
            Some(
                AccessTokenAuthorityConfig::new(
                    format!("http://{address}/authorize"),
                    "dep_01JTESTDEPLOYMENT",
                )
                .unwrap(),
            ),
            std::time::Duration::from_secs(2),
        );

        let issued = auth
            .exchange_access_token("account-access-token")
            .await
            .unwrap();
        server.abort();

        for operation in [Operation::ProjectRead, Operation::ProjectWrite] {
            assert!(matches!(
                auth.authorize_project(
                    &deployment_headers(&issued.credential),
                    operation,
                    Some("user-123"),
                )
                .await
                .unwrap(),
                Identity::ActingUser { subject, .. } if subject == "user-123"
            ));
        }
    }

    #[tokio::test]
    async fn rejects_tampered_and_cross_deployment_credentials() {
        let auth = ApplicationAuth::with_deployment_credential_auth(
            "test-deployment-admin-key",
            "dep_01JTESTDEPLOYMENT",
            Arc::new(StaticAccessTokenAuth {
                expected: "account-access-token",
                operations: vec![Operation::ProjectRead],
            }),
        );
        let issued = auth
            .exchange_access_token("account-access-token")
            .await
            .unwrap();
        let mut tampered = issued.credential.clone();
        tampered.push('x');
        assert!(matches!(
            auth.authorize_token(&tampered, Operation::ProjectRead)
                .await,
            Err(ApiError::Unauthorized)
        ));

        let other_deployment = ApplicationAuth::with_deployment_credential_auth(
            "test-deployment-admin-key",
            "dep_01JOTHERDEPLOYMENT",
            Arc::new(RejectingAccessTokenAuth),
        );
        assert!(matches!(
            other_deployment
                .authorize_token(&issued.credential, Operation::ProjectRead)
                .await,
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn deployment_credentials_expire_after_the_short_lived_window() {
        let issuer = DeploymentCredentialIssuer::new(
            Arc::from("test-deployment-admin-key"),
            "dep_01JTESTDEPLOYMENT".to_string(),
        );
        let identity = Identity::ActingUser {
            subject: "user-123".to_string(),
            issuer: "test-authority".to_string(),
            operations: vec![Operation::ProjectRead],
        };
        let issued_at = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let issued = issuer.issue(&identity, issued_at).unwrap();

        assert!(issuer
            .verify(
                &issued.credential,
                issued_at + DEPLOYMENT_CREDENTIAL_LIFETIME - Duration::from_secs(1),
            )
            .is_ok());
        assert!(matches!(
            issuer.verify(
                &issued.credential,
                issued_at + DEPLOYMENT_CREDENTIAL_LIFETIME,
            ),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn acting_users_are_bound_to_projects_by_canonical_user_id() {
        let identity = Identity::ActingUser {
            subject: "user-123".to_string(),
            issuer: "test-authority".to_string(),
            operations: vec![Operation::ProjectWrite],
        };

        assert!(
            require_project_operation(&identity, Operation::ProjectRead, Some("user-123")).is_ok()
        );
        assert!(matches!(
            require_project_operation(&identity, Operation::ProjectRead, Some("user-456")),
            Err(ApiError::Forbidden)
        ));
        assert!(matches!(
            require_project_operation(&identity, Operation::ProjectRead, None),
            Err(ApiError::Forbidden)
        ));
        assert!(require_project_operation(
            &Identity::DeploymentAdmin,
            Operation::ProjectWrite,
            None,
        )
        .is_ok());
    }

    struct RejectingAccessTokenAuth;

    impl AccessTokenAuth for RejectingAccessTokenAuth {
        fn is_authorized<'a>(
            &'a self,
            _access_token: &'a str,
            _operation: Operation,
        ) -> AccessTokenAuthFuture<'a> {
            Box::pin(async { Err(ApiError::Forbidden) })
        }
    }

    struct StaticAccessTokenAuth {
        expected: &'static str,
        operations: Vec<Operation>,
    }

    impl AccessTokenAuth for StaticAccessTokenAuth {
        fn is_authorized<'a>(
            &'a self,
            access_token: &'a str,
            _operation: Operation,
        ) -> AccessTokenAuthFuture<'a> {
            Box::pin(async move {
                if access_token != self.expected {
                    return Err(ApiError::Forbidden);
                }
                Ok(Identity::ActingUser {
                    subject: "user-123".to_string(),
                    issuer: "test-authority".to_string(),
                    operations: self.operations.clone(),
                })
            })
        }
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        authorization_headers("Bearer", token)
    }

    fn deployment_headers(token: &str) -> HeaderMap {
        authorization_headers("Vifu", token)
    }

    fn authorization_headers(scheme: &str, token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("{scheme} {token}")).unwrap(),
        );
        headers
    }
}
