#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::new_id;
use crate::native::model::types::NativeImage;

pub const MAX_NATIVE_IMAGES: usize = 8;
pub const MAX_NATIVE_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const ATTACHMENTS_DIR_NAME: &str = "attachments";
const MAX_STAGED_NAME_LEN: usize = 80;

#[derive(Debug, Default)]
pub struct NativeImageLoad {
    pub images: Vec<NativeImage>,
    pub loaded_paths: Vec<String>,
    pub missing: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn load_native_images(paths: Option<&[String]>) -> NativeImageLoad {
    let mut seen = HashSet::new();
    let mut loaded = NativeImageLoad::default();
    for raw in paths.unwrap_or_default() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        let path = Path::new(trimmed);
        if !path.is_file() {
            loaded.missing.push(trimmed.to_string());
            continue;
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| trimmed.to_string());
        if loaded.images.len() >= MAX_NATIVE_IMAGES {
            loaded
                .skipped
                .push(format!("{name}（最多 {MAX_NATIVE_IMAGES} 张）"));
            continue;
        }
        let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        if size > MAX_NATIVE_IMAGE_BYTES {
            loaded.skipped.push(format!("{name}（超过 8MB）"));
            continue;
        }
        match fs::read(path) {
            Ok(bytes) => {
                loaded.images.push(NativeImage {
                    name,
                    mime_type: image_mime_type(path).to_string(),
                    data_base64: BASE64.encode(bytes),
                });
                loaded.loaded_paths.push(trimmed.to_string());
            }
            Err(_) => loaded.missing.push(trimmed.to_string()),
        }
    }
    loaded
}

pub fn image_log_lines(loaded: &NativeImageLoad) -> Vec<String> {
    let mut lines = Vec::new();
    if !loaded.images.is_empty() {
        let names = loaded
            .images
            .iter()
            .enumerate()
            .map(|(index, image)| format!("{}. {}", index + 1, image.name))
            .collect::<Vec<_>>()
            .join("\n");
        lines.push(format!("附带图片: {} 张\n{names}", loaded.images.len()));
    }
    for path in &loaded.missing {
        lines.push(format!("跳过缺失图片: {path}"));
    }
    for reason in &loaded.skipped {
        lines.push(format!("跳过图片: {reason}"));
    }
    lines
}

fn image_mime_type(path: &Path) -> &'static str {
    match image_extension(path).as_deref() {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    }
}

fn image_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

fn is_allowed_image_path(path: &Path) -> bool {
    matches!(
        image_extension(path).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

pub fn attachments_dir(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join(ATTACHMENTS_DIR_NAME)
}

pub fn sanitize_attachment_name(name: &str) -> String {
    let file_name = Path::new(name)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let cleaned: String = file_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').trim_matches('_');
    let cleaned = if trimmed.is_empty() {
        "image.png".to_string()
    } else {
        trimmed.to_string()
    };
    if cleaned.chars().count() > MAX_STAGED_NAME_LEN {
        cleaned.chars().take(MAX_STAGED_NAME_LEN).collect()
    } else {
        cleaned
    }
}

pub fn stage_image_bytes(
    app_config_dir: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    if bytes.len() as u64 > MAX_NATIVE_IMAGE_BYTES {
        return Err(format!("{}（超过 8MB）", sanitize_attachment_name(name)));
    }
    let dir = attachments_dir(app_config_dir);
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建附件目录: {error}"))?;
    let file_name = format!("{}_{}", new_id(), sanitize_attachment_name(name));
    let path = dir.join(file_name);
    fs::write(&path, bytes).map_err(|error| format!("写入附件失败: {error}"))?;
    Ok(path)
}

pub fn stage_image_from_path(app_config_dir: &Path, source: &Path) -> Result<PathBuf, String> {
    if !source.is_file() {
        return Err(format!("图片不存在: {}", source.display()));
    }
    if !is_allowed_image_path(source) {
        return Err(format!("不支持的图片类型: {}", source.display()));
    }
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image.png".to_string());
    let bytes = fs::read(source).map_err(|error| format!("读取图片失败: {error}"))?;
    stage_image_bytes(app_config_dir, &name, &bytes)
}

pub fn cleanup_staged_loaded_images(loaded: &NativeImageLoad) {
    for raw in &loaded.loaded_paths {
        let path = Path::new(raw);
        let is_attachment = path
            .parent()
            .and_then(|dir| dir.file_name())
            .is_some_and(|name| name == ATTACHMENTS_DIR_NAME);
        if is_attachment && path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn delete_staged_images(app_config_dir: &Path, paths: &[String]) -> Result<(), String> {
    let root = attachments_dir(app_config_dir);
    for raw in paths {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = Path::new(trimmed);
        if path.starts_with(&root) && path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

#[tauri::command]
pub fn stage_composer_image(
    app: AppHandle,
    name: String,
    data_base64: String,
) -> Result<String, String> {
    let dir = app_config_dir(&app)?;
    let bytes = BASE64
        .decode(data_base64.trim())
        .map_err(|_| "图片数据无效".to_string())?;
    let path = stage_image_bytes(&dir, &name, &bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn stage_composer_image_from_path(
    app: AppHandle,
    source_path: String,
) -> Result<String, String> {
    let dir = app_config_dir(&app)?;
    let path = stage_image_from_path(&dir, Path::new(source_path.trim()))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn delete_composer_images(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    let dir = app_config_dir(&app)?;
    delete_staged_images(&dir, &paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_png() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-ai-native-img-{stamp}.png"));
        fs::write(&path, b"\x89PNG\r\n").expect("write png");
        path
    }

    #[test]
    fn loads_existing_file_and_skips_missing() {
        let path = temp_png();
        let loaded = load_native_images(Some(&[
            path.to_string_lossy().into_owned(),
            "/definitely/missing/native-image.png".to_string(),
        ]));
        assert_eq!(loaded.images.len(), 1);
        assert_eq!(loaded.images[0].mime_type, "image/png");
        assert!(!loaded.images[0].data_base64.is_empty());
        assert_eq!(loaded.missing.len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn data_url_uses_mime_and_payload() {
        let image = NativeImage {
            name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "QQ==".to_string(),
        };
        assert_eq!(image.data_url(), "data:image/png;base64,QQ==");
    }

    fn staging_root() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("noxcode-composer-img-{stamp}"));
        fs::create_dir_all(&dir).expect("staging root");
        dir
    }

    #[test]
    fn stages_bytes_and_only_deletes_attachments() {
        let root = staging_root();
        let staged = stage_image_bytes(&root, "shot.png", b"\x89PNG\r\n").expect("stage");
        assert!(staged.starts_with(attachments_dir(&root)));
        assert!(staged.is_file());

        let outside = root.join("keep.png");
        fs::write(&outside, b"keep").expect("outside");
        delete_staged_images(
            &root,
            &[
                staged.to_string_lossy().into_owned(),
                outside.to_string_lossy().into_owned(),
            ],
        )
        .expect("delete");
        assert!(!staged.exists());
        assert!(outside.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_removes_loaded_attachment_files_only() {
        let root = staging_root();
        let staged = stage_image_bytes(&root, "keep-me.png", b"\x89PNG\r\n").expect("stage");
        let loaded = load_native_images(Some(&[staged.to_string_lossy().into_owned()]));
        assert_eq!(loaded.images.len(), 1);
        cleanup_staged_loaded_images(&loaded);
        assert!(!staged.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copies_allowed_path_and_rejects_other_types() {
        let root = staging_root();
        let source = root.join("source.webp");
        fs::write(&source, b"RIFF").expect("source");
        let staged = stage_image_from_path(&root, &source).expect("copy");
        assert!(staged.is_file());

        let text = root.join("note.txt");
        fs::write(&text, b"hi").expect("text");
        assert!(stage_image_from_path(&root, &text).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
