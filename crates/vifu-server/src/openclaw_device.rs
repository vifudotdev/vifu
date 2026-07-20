use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vifu_core::openclaw_rpc::OpenClawDeviceSigner;

const MAX_IDENTITY_FILE_BYTES: u64 = 64 * 1024;
static IDENTITY_IO_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredIdentity {
    version: u8,
    device_id: String,
    public_key: String,
    private_key_pkcs8: String,
    created_at_ms: u64,
}

pub struct OpenClawDeviceIdentity {
    device_id: String,
    public_key: String,
    private_key_pkcs8: Vec<u8>,
    key_pair: Ed25519KeyPair,
}

impl OpenClawDeviceSigner for OpenClawDeviceIdentity {
    fn device_id(&self) -> &str {
        &self.device_id
    }

    fn public_key(&self) -> &str {
        &self.public_key
    }

    fn sign(&self, payload: &str) -> Result<String, String> {
        Ok(URL_SAFE_NO_PAD.encode(self.key_pair.sign(payload.as_bytes()).as_ref()))
    }
}

pub fn load_or_create(
    home_dir: &Path,
    project_slug: &str,
    provider_key: &str,
) -> Result<OpenClawDeviceIdentity, String> {
    let _guard = IDENTITY_IO_LOCK
        .lock()
        .map_err(|_| "OpenClaw device identity lock is unavailable".to_string())?;
    let path = identity_path(home_dir, project_slug, provider_key);
    let parent = path
        .parent()
        .ok_or_else(|| "OpenClaw device identity path is invalid".to_string())?;
    create_private_dir(parent)?;
    if path.exists() {
        set_private_file_permissions(&path)?;
        return load(&path);
    }

    let identity = generate()?;
    let stored = StoredIdentity {
        version: 1,
        device_id: identity.device_id.clone(),
        public_key: identity.public_key.clone(),
        private_key_pkcs8: URL_SAFE_NO_PAD.encode(&identity.private_key_pkcs8),
        created_at_ms: now_ms()?,
    };
    write_new(&path, &stored)?;
    Ok(identity)
}

fn identity_path(home_dir: &Path, project_slug: &str, provider_key: &str) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(b"vifu-openclaw-device-identity-v1");
    digest.update([0]);
    digest.update(project_slug.as_bytes());
    digest.update([0]);
    digest.update(provider_key.as_bytes());
    let digest = digest.finalize();
    let file_name = format!("{}.json", hex_digest(&digest));
    home_dir.join("identities").join("openclaw").join(file_name)
}

fn generate() -> Result<OpenClawDeviceIdentity, String> {
    let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| "could not generate OpenClaw device identity".to_string())?;
    identity_from_pkcs8(document.as_ref())
}

fn load(path: &Path) -> Result<OpenClawDeviceIdentity, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not inspect OpenClaw device identity: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_IDENTITY_FILE_BYTES {
        return Err("OpenClaw device identity file is invalid".to_string());
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read OpenClaw device identity: {error}"))?;
    let stored = serde_json::from_slice::<StoredIdentity>(&bytes)
        .map_err(|_| "OpenClaw device identity file is invalid".to_string())?;
    if stored.version != 1 {
        return Err("OpenClaw device identity version is unsupported".to_string());
    }
    let private_key = URL_SAFE_NO_PAD
        .decode(&stored.private_key_pkcs8)
        .map_err(|_| "OpenClaw device identity private key is invalid".to_string())?;
    let identity = identity_from_pkcs8(&private_key)?;
    if identity.device_id != stored.device_id || identity.public_key != stored.public_key {
        return Err("OpenClaw device identity key material does not match".to_string());
    }
    Ok(identity)
}

fn identity_from_pkcs8(pkcs8: &[u8]) -> Result<OpenClawDeviceIdentity, String> {
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|_| "OpenClaw device identity private key is invalid".to_string())?;
    let public_key_bytes = key_pair.public_key().as_ref();
    let public_key = URL_SAFE_NO_PAD.encode(public_key_bytes);
    let digest = Sha256::digest(public_key_bytes);
    let device_id = hex_digest(&digest);
    Ok(OpenClawDeviceIdentity {
        device_id,
        public_key,
        private_key_pkcs8: pkcs8.to_vec(),
        key_pair,
    })
}

fn write_new(path: &Path, value: &StoredIdentity) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("could not encode OpenClaw device identity: {error}"))?;
    bytes.push(b'\n');
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "OpenClaw device identity path is invalid".to_string())?;
    let temporary_path = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary_path)
            .map_err(|error| format!("could not create OpenClaw device identity: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("could not write OpenClaw device identity: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("could not persist OpenClaw device identity: {error}"))?;
        fs::hard_link(&temporary_path, path)
            .map_err(|error| format!("could not install OpenClaw device identity: {error}"))?;
        set_private_file_permissions(path)
    })();
    let _ = fs::remove_file(&temporary_path);
    result
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("could not create OpenClaw identity directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not secure OpenClaw identity directory: {error}"))?;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not secure OpenClaw device identity: {error}"))?;
    }
    Ok(())
}

fn now_ms() -> Result<u64, String> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_millis();
    u64::try_from(milliseconds).map_err(|_| "system clock value is too large".to_string())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[cfg(test)]
mod tests {
    use std::fs;

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use ring::signature::{UnparsedPublicKey, ED25519};
    use serde_json::Value;
    use uuid::Uuid;
    use vifu_core::openclaw_rpc::OpenClawDeviceSigner;

    use super::{identity_path, load_or_create};

    #[test]
    fn persists_and_reloads_a_signing_identity() {
        let home = test_home();
        let first = load_or_create(&home, "project-a", "openclaw-local").unwrap();
        let first_id = first.device_id().to_string();
        let signature = URL_SAFE_NO_PAD
            .decode(first.sign("payload").unwrap())
            .unwrap();
        let public_key = URL_SAFE_NO_PAD.decode(first.public_key()).unwrap();
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(b"payload", &signature)
            .unwrap();

        let second = load_or_create(&home, "project-a", "openclaw-local").unwrap();
        assert_eq!(second.device_id(), first_id);
        assert_eq!(second.public_key(), first.public_key());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(identity_path(&home, "project-a", "openclaw-local"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn rejects_tampered_identity_metadata() {
        let home = test_home();
        load_or_create(&home, "project-a", "openclaw-local").unwrap();
        let path = identity_path(&home, "project-a", "openclaw-local");
        let mut stored = serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap();
        stored["deviceId"] = Value::String("0".repeat(64));
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();

        let error = load_or_create(&home, "project-a", "openclaw-local")
            .err()
            .unwrap();
        assert!(error.contains("does not match"));
        fs::remove_dir_all(home).unwrap();
    }

    fn test_home() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vifu-openclaw-identity-{}", Uuid::new_v4()))
    }
}
