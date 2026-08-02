use std::fmt;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MACHINE_ID_PREFIX: &str = "machine-";
const MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1_000;

/// Stable installation identity used to prove Gateway ownership.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineIdentity {
    pub machine_id: String,
    pub public_key: String,
    private_key_pkcs8: String,
}

impl fmt::Debug for MachineIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MachineIdentity")
            .field("machine_id", &self.machine_id)
            .field("public_key", &self.public_key)
            .field("private_key_pkcs8", &"[REDACTED]")
            .finish()
    }
}

impl MachineIdentity {
    pub fn generate() -> Result<Self, String> {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .map_err(|_| "could not generate Gateway machine identity".to_string())?;
        Self::from_pkcs8(pkcs8.as_ref())
    }

    pub fn from_encoded_private_key(private_key_pkcs8: &str) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(private_key_pkcs8.trim())
            .map_err(|_| "Gateway machine private key is invalid".to_string())?;
        Self::from_pkcs8(&bytes)
    }

    fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, String> {
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8)
            .map_err(|_| "Gateway machine private key is invalid".to_string())?;
        let public_key = URL_SAFE_NO_PAD.encode(key_pair.public_key().as_ref());
        Ok(Self {
            machine_id: machine_id_for_public_key(key_pair.public_key().as_ref()),
            public_key,
            private_key_pkcs8: URL_SAFE_NO_PAD.encode(pkcs8),
        })
    }

    pub fn encoded_private_key(&self) -> &str {
        &self.private_key_pkcs8
    }

    pub fn sign(&self, payload: &[u8]) -> Result<String, String> {
        let private_key = URL_SAFE_NO_PAD
            .decode(&self.private_key_pkcs8)
            .map_err(|_| "Gateway machine private key is invalid".to_string())?;
        let key_pair = Ed25519KeyPair::from_pkcs8(&private_key)
            .map_err(|_| "Gateway machine private key is invalid".to_string())?;
        Ok(URL_SAFE_NO_PAD.encode(key_pair.sign(payload).as_ref()))
    }

    pub fn validate(&self) -> Result<(), String> {
        let rebuilt = Self::from_encoded_private_key(&self.private_key_pkcs8)?;
        if rebuilt.machine_id != self.machine_id || rebuilt.public_key != self.public_key {
            return Err("Gateway machine identity does not match its private key".to_string());
        }
        Ok(())
    }
}

pub fn verify_machine_signature(
    machine_id: &str,
    public_key: &str,
    signature: &str,
    payload: &[u8],
) -> Result<(), String> {
    let public_key = URL_SAFE_NO_PAD
        .decode(public_key.trim())
        .map_err(|_| "Gateway machine public key is invalid".to_string())?;
    if machine_id_for_public_key(&public_key) != machine_id {
        return Err("Gateway machine id does not match its public key".to_string());
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature.trim())
        .map_err(|_| "Gateway machine signature is invalid".to_string())?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(payload, &signature)
        .map_err(|_| "Gateway machine signature is invalid".to_string())
}

pub fn machine_id_for_public_key(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    format!("{MACHINE_ID_PREFIX}{}", hex(&digest))
}

pub fn validate_signed_at(now_ms: u64, signed_at_ms: u64) -> Result<(), String> {
    if now_ms.abs_diff(signed_at_ms) > MAX_CLOCK_SKEW_MS {
        return Err(
            "Gateway machine signature timestamp is outside the allowed window".to_string(),
        );
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identity_round_trips_and_verifies_signatures() {
        let identity = MachineIdentity::generate().unwrap();
        let restored =
            MachineIdentity::from_encoded_private_key(identity.encoded_private_key()).unwrap();
        assert_eq!(restored.machine_id, identity.machine_id);
        assert_eq!(restored.public_key, identity.public_key);
        let payload = b"vifu gateway challenge";
        let signature = restored.sign(payload).unwrap();
        verify_machine_signature(
            &restored.machine_id,
            &restored.public_key,
            &signature,
            payload,
        )
        .unwrap();
    }

    #[test]
    fn signature_rejects_another_machine() {
        let signer = MachineIdentity::generate().unwrap();
        let other = MachineIdentity::generate().unwrap();
        let signature = signer.sign(b"payload").unwrap();
        assert!(verify_machine_signature(
            &other.machine_id,
            &other.public_key,
            &signature,
            b"payload",
        )
        .is_err());
    }
}
