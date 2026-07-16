use axum::http::HeaderMap;
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::ApiError;

pub fn require_admin(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, expected) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

pub fn require_agent_gateway(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    require_admin(headers, expected)
}

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn hash_api_key(value: &str, pepper: &str) -> Vec<u8> {
    let mut digest = Sha256::new();
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
    use super::{constant_time_eq, decrypt_secret_json, encrypt_secret_json, hash_api_key};

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
}
