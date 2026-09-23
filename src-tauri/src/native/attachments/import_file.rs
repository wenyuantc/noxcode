use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::native::media_error::{
    MediaError, CHUNK_CONFLICT, CHUNK_OUT_OF_ORDER, CHUNK_TOO_LARGE, DECLARED_SIZE_EXCEEDED,
    IMPORT_INVALID, NOT_FOUND,
};
use crate::native::media_limits::MAX_CHUNK_BYTES;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StagingMeta {
    pub declared_size: u64,
    pub confirmed: u64,
    pub original_name: String,
    pub mime: String,
    pub draft_id: String,
    pub status: String,
    pub attachment_id: Option<String>,
}

pub(crate) struct StagingImport {
    dir: PathBuf,
    pub meta: StagingMeta,
}

impl StagingImport {
    pub(crate) fn create(
        root: &Path,
        import_id: &str,
        declared_size: u64,
        original_name: &str,
        mime: &str,
        draft_id: &str,
    ) -> Result<Self, MediaError> {
        let dir = staging_dir(root, import_id)?;
        if dir.exists() {
            return Self::open(root, import_id);
        }
        fs::create_dir_all(&dir)
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        let meta = StagingMeta {
            declared_size,
            confirmed: 0,
            original_name: original_name.to_string(),
            mime: mime.to_string(),
            draft_id: draft_id.to_string(),
            status: "importing".to_string(),
            attachment_id: None,
        };
        let staging = Self { dir, meta };
        staging.save()?;
        Ok(staging)
    }

    pub(crate) fn open(root: &Path, import_id: &str) -> Result<Self, MediaError> {
        let dir = staging_dir(root, import_id)?;
        let raw = fs::read(dir.join("state.json"))
            .map_err(|_| MediaError::new(NOT_FOUND, "导入不存在"))?;
        let meta = serde_json::from_slice(&raw)
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        Ok(Self { dir, meta })
    }

    pub(crate) fn append(&mut self, offset: u64, bytes: &[u8]) -> Result<u64, MediaError> {
        if self.meta.status != "importing" {
            return Err(MediaError::new(IMPORT_INVALID, "导入已结束"));
        }
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err(MediaError::new(CHUNK_TOO_LARGE, "分块超过 1 MiB"));
        }
        if offset > self.meta.confirmed {
            return Err(MediaError::new(CHUNK_OUT_OF_ORDER, "分块偏移跳跃"));
        }
        if offset < self.meta.confirmed {
            let existing = self.confirmed_slice(offset, bytes.len())?;
            if existing == bytes {
                return Ok(self.meta.confirmed);
            }
            return Err(MediaError::new(CHUNK_CONFLICT, "相同偏移的内容不一致"));
        }
        let next = offset + bytes.len() as u64;
        if next > self.meta.declared_size {
            return Err(MediaError::new(
                DECLARED_SIZE_EXCEEDED,
                "分块超过声明总大小",
            ));
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.partial_path())
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        file.write_all(bytes)
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        file.sync_all()
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        self.meta.confirmed = next;
        self.save()?;
        Ok(self.meta.confirmed)
    }

    pub(crate) fn confirmed_bytes(&self) -> Result<Vec<u8>, MediaError> {
        if self.meta.confirmed == 0 {
            return Ok(Vec::new());
        }
        let bytes = fs::read(self.partial_path())
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        if bytes.len() as u64 != self.meta.confirmed {
            return Err(MediaError::new(IMPORT_INVALID, "已确认前缀长度不一致"));
        }
        Ok(bytes)
    }

    pub(crate) fn save_status(
        &mut self,
        status: &str,
        attachment_id: Option<String>,
    ) -> Result<(), MediaError> {
        self.meta.status = status.to_string();
        self.meta.attachment_id = attachment_id;
        self.save()
    }

    pub(crate) fn dir_id(&self) -> String {
        self.dir
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub(crate) fn remove_partial(&self) -> Result<(), MediaError> {
        let path = self.partial_path();
        if path.exists() {
            fs::remove_file(path)
                .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        }
        Ok(())
    }

    fn confirmed_slice(&self, offset: u64, len: usize) -> Result<Vec<u8>, MediaError> {
        let bytes = self.confirmed_bytes()?;
        let start = offset as usize;
        let end = start + len;
        if end > bytes.len() {
            return Err(MediaError::new(CHUNK_CONFLICT, "重试块超出已确认前缀"));
        }
        Ok(bytes[start..end].to_vec())
    }

    fn save(&self) -> Result<(), MediaError> {
        let raw = serde_json::to_vec(&self.meta)
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        let path = self.dir.join("state.json");
        let tmp = self.dir.join("state.json.tmp");
        fs::write(&tmp, raw).map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        fs::rename(tmp, path)
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        Ok(())
    }

    fn partial_path(&self) -> PathBuf {
        self.dir.join("partial.bin")
    }
}

pub(crate) fn staging_dir(root: &Path, import_id: &str) -> Result<PathBuf, MediaError> {
    if import_id.is_empty()
        || import_id.contains('/')
        || import_id.contains('\\')
        || import_id.contains("..")
    {
        return Err(MediaError::new(IMPORT_INVALID, "导入标识非法"));
    }
    Ok(root.join("staging").join(import_id))
}

pub(crate) fn relative_object_path(id: &str) -> String {
    let compact: String = id.chars().filter(|ch| *ch != '-').collect();
    let shard = if compact.len() >= 2 {
        &compact[..2]
    } else {
        "00"
    };
    format!("objects/{shard}/{id}.bin")
}
