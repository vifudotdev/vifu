use axum::http::HeaderMap;
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

pub fn require_connector(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
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

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, hash_api_key};

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
}
