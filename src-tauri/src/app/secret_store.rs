use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use uuid::Uuid;

use crate::app::shared::now_sqlite;

const SECRET_INDEX_FILE_NAME: &str = "ssh-secret-index.json";
const SECRET_INDEX_VERSION: u32 = 2;
const KEYRING_SERVICE: &str = "noxcode-ssh";

const SECRET_SERVICE_UNAVAILABLE_MSG: &str =
    "系统凭据服务不可用，无法保存 SSH 密码。请安装/启用 Secret Service（如 gnome-keyring）后重试";

trait SecretBackend: Send + Sync {
    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String>;
    fn get(&self, secret_ref: &str) -> Result<Option<String>, String>;
    fn delete(&self, secret_ref: &str) -> Result<(), String>;
}

#[cfg(test)]
struct MemoryBackend {
    store: Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl MemoryBackend {
    fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl SecretBackend for MemoryBackend {
    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String> {
        let mut guard = self
            .store
            .lock()
            .map_err(|_| "内存密钥后端锁定失败".to_string())?;
        guard.insert(secret_ref.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, secret_ref: &str) -> Result<Option<String>, String> {
        let guard = self
            .store
            .lock()
            .map_err(|_| "内存密钥后端锁定失败".to_string())?;
        Ok(guard.get(secret_ref).cloned())
    }

    fn delete(&self, secret_ref: &str) -> Result<(), String> {
        let mut guard = self
            .store
            .lock()
            .map_err(|_| "内存密钥后端锁定失败".to_string())?;
        guard.remove(secret_ref);
        Ok(())
    }
}

struct KeyringBackend {
    service: &'static str,
}

impl KeyringBackend {
    fn new() -> Self {
        Self {
            service: KEYRING_SERVICE,
        }
    }

    fn entry(&self, secret_ref: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(self.service, secret_ref).map_err(map_keyring_error)
    }
}

impl SecretBackend for KeyringBackend {
    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String> {
        self.entry(secret_ref)?
            .set_password(value)
            .map_err(map_keyring_error)
    }

    fn get(&self, secret_ref: &str) -> Result<Option<String>, String> {
        match self.entry(secret_ref)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring_error(error)),
        }
    }

    fn delete(&self, secret_ref: &str) -> Result<(), String> {
        match self.entry(secret_ref)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring_error(error)),
        }
    }
}

fn looks_like_missing_secret_service(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("secret service")
        || lower.contains("no secret service")
        || lower.contains("org.freedesktop.secrets")
        || (lower.contains("dbus")
            && (lower.contains("connect")
                || lower.contains("unavailable")
                || lower.contains("failed")))
        || lower.contains("platform secure storage failure")
        || lower.contains("couldn't access platform secure storage")
}

fn map_keyring_error(error: keyring::Error) -> String {
    match error {
        keyring::Error::NoStorageAccess(err) => {
            let detail = err.to_string();
            if cfg!(target_os = "linux") || looks_like_missing_secret_service(&detail) {
                SECRET_SERVICE_UNAVAILABLE_MSG.to_string()
            } else {
                format!("系统凭据库操作失败: 无法访问凭据存储: {detail}")
            }
        }
        keyring::Error::PlatformFailure(err) => {
            let detail = err.to_string();
            if looks_like_missing_secret_service(&detail) {
                SECRET_SERVICE_UNAVAILABLE_MSG.to_string()
            } else {
                format!("系统凭据库操作失败: {detail}")
            }
        }
        other => {
            let message = other.to_string();
            if looks_like_missing_secret_service(&message) {
                SECRET_SERVICE_UNAVAILABLE_MSG.to_string()
            } else {
                format!("系统凭据库操作失败: {message}")
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct SecretMeta {
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SecretIndexDocument {
    #[serde(default = "default_index_version")]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, SecretMeta>,
}

fn default_index_version() -> u32 {
    SECRET_INDEX_VERSION
}

impl Default for SecretIndexDocument {
    fn default() -> Self {
        Self {
            version: SECRET_INDEX_VERSION,
            entries: HashMap::new(),
        }
    }
}

fn index_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SECRET_INDEX_FILE_NAME)
}

fn tighten_secret_store_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("设置 secret store 权限失败: {error}"))?;
    }

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

fn load_index_document(config_dir: &Path) -> Result<SecretIndexDocument, String> {
    let path = index_path(config_dir);
    if !path.exists() {
        return Ok(SecretIndexDocument::default());
    }

    let raw = fs::read_to_string(&path).map_err(|error| format!("读取密钥索引失败: {error}"))?;
    let mut document: SecretIndexDocument =
        serde_json::from_str(&raw).map_err(|error| format!("解析密钥索引失败: {error}"))?;
    if document.version == 0 {
        document.version = SECRET_INDEX_VERSION;
    }
    Ok(document)
}

fn save_index_document(config_dir: &Path, document: &SecretIndexDocument) -> Result<(), String> {
    let path = index_path(config_dir);
    fs::create_dir_all(config_dir).map_err(|error| format!("创建密钥索引目录失败: {error}"))?;
    let raw = serde_json::to_string_pretty(document)
        .map_err(|error| format!("序列化密钥索引失败: {error}"))?;

    let tmp_path = config_dir.join(format!(
        ".{SECRET_INDEX_FILE_NAME}.{}.tmp",
        std::process::id()
    ));
    fs::write(&tmp_path, raw.as_bytes()).map_err(|error| format!("写入密钥索引失败: {error}"))?;
    if let Err(error) = fs::rename(&tmp_path, &path) {
        let _ = fs::remove_file(&path);
        fs::rename(&tmp_path, &path).map_err(|rename_error| {
            let _ = fs::remove_file(&tmp_path);
            format!("写入密钥索引失败: {error}; 重试: {rename_error}")
        })?;
    }
    tighten_secret_store_permissions(&path)?;
    Ok(())
}

fn store_secret_value_with(
    config_dir: &Path,
    backend: &dyn SecretBackend,
    value: Option<&str>,
    replace_ref: Option<&str>,
) -> Result<Option<String>, String> {
    let normalized = value.map(str::trim).filter(|value| !value.is_empty());
    let mut index = load_index_document(config_dir)?;

    let replace_ref = replace_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(ref replace_ref) = replace_ref {
        index.entries.remove(replace_ref);
    }

    let Some(value) = normalized else {
        save_index_document(config_dir, &index)?;
        if let Some(replace_ref) = replace_ref {
            let _ = backend.delete(&replace_ref);
        }
        return Ok(None);
    };

    let secret_ref = format!("ssh-secret-{}", Uuid::new_v4());
    let now = now_sqlite();
    backend.set(&secret_ref, value)?;
    index.entries.insert(
        secret_ref.clone(),
        SecretMeta {
            created_at: now.clone(),
            updated_at: now,
        },
    );
    index.version = SECRET_INDEX_VERSION;
    if let Err(error) = save_index_document(config_dir, &index) {
        let _ = backend.delete(&secret_ref);
        return Err(error);
    }
    if let Some(replace_ref) = replace_ref {
        let _ = backend.delete(&replace_ref);
    }
    Ok(Some(secret_ref))
}

fn resolve_secret_value_with(
    config_dir: &Path,
    backend: &dyn SecretBackend,
    secret_ref: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(secret_ref) = secret_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let index = load_index_document(config_dir)?;
    if !index.entries.contains_key(secret_ref) {
        return Ok(None);
    }

    match backend.get(secret_ref)? {
        Some(value) => Ok(Some(value)),
        None => Err(format!(
            "密钥索引存在但系统凭据库中缺少对应条目（可能已损坏）: {secret_ref}"
        )),
    }
}

fn delete_secret_value_with(
    config_dir: &Path,
    backend: &dyn SecretBackend,
    secret_ref: Option<&str>,
) -> Result<(), String> {
    let Some(secret_ref) = secret_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };

    backend.delete(secret_ref)?;
    let mut index = load_index_document(config_dir)?;
    if index.entries.remove(secret_ref).is_some() {
        save_index_document(config_dir, &index)?;
    }
    Ok(())
}

fn sweep_orphan_secret_refs_with(
    config_dir: &Path,
    backend: &dyn SecretBackend,
    active_refs: &HashSet<String>,
) -> Result<usize, String> {
    let mut index = load_index_document(config_dir)?;
    let orphans: Vec<String> = index
        .entries
        .keys()
        .filter(|secret_ref| !active_refs.contains(*secret_ref))
        .cloned()
        .collect();

    let removed = orphans.len();
    for secret_ref in &orphans {
        backend.delete(secret_ref)?;
        index.entries.remove(secret_ref);
    }

    if removed > 0 {
        save_index_document(config_dir, &index)?;
    }
    Ok(removed)
}

pub(crate) struct SecretStore {
    config_dir: PathBuf,
    backend: Box<dyn SecretBackend>,
}

impl SecretStore {
    pub(crate) fn for_app<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        let config_dir = app
            .path()
            .app_config_dir()
            .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
        Ok(Self {
            config_dir,
            backend: Box::new(KeyringBackend::new()),
        })
    }

    #[cfg(test)]
    pub(crate) fn in_memory(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            backend: Box::new(MemoryBackend::new()),
        }
    }

    pub(crate) fn store(
        &self,
        value: Option<&str>,
        replace_ref: Option<&str>,
    ) -> Result<Option<String>, String> {
        store_secret_value_with(&self.config_dir, self.backend.as_ref(), value, replace_ref)
    }

    pub(crate) fn resolve(&self, secret_ref: Option<&str>) -> Result<Option<String>, String> {
        resolve_secret_value_with(&self.config_dir, self.backend.as_ref(), secret_ref)
    }

    pub(crate) fn delete(&self, secret_ref: Option<&str>) -> Result<(), String> {
        delete_secret_value_with(&self.config_dir, self.backend.as_ref(), secret_ref)
    }

    pub(crate) fn sweep_orphans(&self, active_refs: &HashSet<String>) -> Result<usize, String> {
        sweep_orphan_secret_refs_with(&self.config_dir, self.backend.as_ref(), active_refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("noxcode-secret-test-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp config dir");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn store_and_resolve_roundtrip() {
        let dir = temp_config_dir();
        let store = SecretStore::in_memory(dir.clone());

        let secret_ref = store
            .store(Some("p@ssw0rd"), None)
            .expect("store")
            .expect("ref");
        assert!(secret_ref.starts_with("ssh-secret-"));

        let resolved = store
            .resolve(Some(&secret_ref))
            .expect("resolve")
            .expect("value");
        assert_eq!(resolved, "p@ssw0rd");

        let raw = fs::read_to_string(index_path(&dir)).expect("read index");
        assert!(!raw.contains("p@ssw0rd"));
        assert!(raw.contains(&secret_ref));
        assert!(!raw.contains("\"value\""));

        cleanup(&dir);
    }

    #[test]
    fn delete_removes_from_backend_and_index() {
        let dir = temp_config_dir();
        let store = SecretStore::in_memory(dir.clone());

        let secret_ref = store
            .store(Some("to-delete"), None)
            .expect("store")
            .expect("ref");

        store.delete(Some(&secret_ref)).expect("delete");

        let resolved = store.resolve(Some(&secret_ref)).expect("resolve");
        assert!(resolved.is_none());

        cleanup(&dir);
    }

    #[test]
    fn replace_ref_deletes_old_value() {
        let dir = temp_config_dir();
        let store = SecretStore::in_memory(dir.clone());

        let old_ref = store
            .store(Some("old-secret"), None)
            .expect("store old")
            .expect("old ref");

        let new_ref = store
            .store(Some("new-secret"), Some(&old_ref))
            .expect("store new")
            .expect("new ref");

        assert_ne!(old_ref, new_ref);
        assert!(store
            .resolve(Some(&old_ref))
            .expect("resolve old")
            .is_none());
        assert_eq!(
            store
                .resolve(Some(&new_ref))
                .expect("resolve new")
                .as_deref(),
            Some("new-secret")
        );

        cleanup(&dir);
    }

    #[test]
    fn store_none_with_replace_clears_password() {
        let dir = temp_config_dir();
        let store = SecretStore::in_memory(dir.clone());

        let old_ref = store
            .store(Some("clear-me"), None)
            .expect("store")
            .expect("ref");

        let result = store.store(None, Some(&old_ref)).expect("clear");
        assert!(result.is_none());
        assert!(store.resolve(Some(&old_ref)).expect("resolve").is_none());

        cleanup(&dir);
    }

    #[test]
    fn sweep_only_removes_orphans() {
        let dir = temp_config_dir();
        let store = SecretStore::in_memory(dir.clone());

        let keep = store
            .store(Some("keep"), None)
            .expect("store keep")
            .expect("keep ref");
        let drop_ref = store
            .store(Some("drop"), None)
            .expect("store drop")
            .expect("drop ref");

        let mut active = HashSet::new();
        active.insert(keep.clone());

        let removed = store.sweep_orphans(&active).expect("sweep");
        assert_eq!(removed, 1);
        assert!(store
            .resolve(Some(&drop_ref))
            .expect("drop resolve")
            .is_none());
        assert_eq!(
            store.resolve(Some(&keep)).expect("resolve keep").as_deref(),
            Some("keep")
        );

        cleanup(&dir);
    }

    #[test]
    fn resolve_missing_index_returns_none() {
        let dir = temp_config_dir();
        let store = SecretStore::in_memory(dir.clone());
        let resolved = store.resolve(Some("ssh-secret-nope")).expect("resolve");
        assert!(resolved.is_none());
        cleanup(&dir);
    }

    #[test]
    fn resolve_index_present_backend_missing_is_error() {
        let dir = temp_config_dir();
        let backend = MemoryBackend::new();

        let secret_ref = store_secret_value_with(&dir, &backend, Some("sync-me"), None)
            .expect("store")
            .expect("ref");

        backend.delete(&secret_ref).expect("delete backend");

        let err = resolve_secret_value_with(&dir, &backend, Some(&secret_ref))
            .expect_err("should error on corrupted state");
        assert!(
            err.contains("损坏") || err.contains("缺少"),
            "unexpected error: {err}"
        );

        cleanup(&dir);
    }

    #[test]
    fn index_serialization_has_no_value_field() {
        let doc = SecretIndexDocument {
            version: SECRET_INDEX_VERSION,
            entries: HashMap::from([(
                "ssh-secret-abc".to_string(),
                SecretMeta {
                    created_at: "t1".to_string(),
                    updated_at: "t2".to_string(),
                },
            )]),
        };
        let raw = serde_json::to_string(&doc).expect("serialize");
        assert!(!raw.contains("\"value\""));
        assert!(raw.contains("\"version\":2") || raw.contains("\"version\": 2"));
    }

    #[test]
    fn looks_like_missing_secret_service_detects_common_messages() {
        assert!(looks_like_missing_secret_service(
            "Platform secure storage failure: dbus connect failed"
        ));
        assert!(looks_like_missing_secret_service(
            "Couldn't access platform secure storage: no secret service"
        ));
        assert!(!looks_like_missing_secret_service(
            "No matching entry found"
        ));
    }
}
