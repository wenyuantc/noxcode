#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::io::Read;
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
        if !is_allowed_media_path(path) {
            loaded.skipped.push(format!("{name}（不支持的附件类型）"));
            continue;
        }
        if loaded.images.len() >= MAX_NATIVE_IMAGES {
            loaded
                .skipped
                .push(format!("{name}（最多 {MAX_NATIVE_IMAGES} 张）"));
            continue;
        }
        let limit = media_byte_limit(path);
        let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        if size > limit {
            loaded.skipped.push(format!("{name}（超过上限）"));
            continue;
        }
        match fs::read(path) {
            Ok(bytes) => {
                if text_mime(image_extension(path).as_deref()).is_some()
                    && std::str::from_utf8(&bytes).is_err()
                {
                    loaded.skipped.push(format!("{name}（不是 UTF-8 文本）"));
                    continue;
                }
                loaded.images.push(NativeImage {
                    name,
                    mime_type: media_mime(path).to_string(),
                    data_base64: BASE64.encode(bytes),
                    attachment_id: attachment_id_of(path),
                    page: None,
                    time_range: None,
                });
                loaded.loaded_paths.push(trimmed.to_string());
            }
            Err(_) => loaded.missing.push(trimmed.to_string()),
        }
    }
    loaded
}

/// Steer must be accepted with all requested attachments or rejected intact.
/// Cleanup is the caller's responsibility after durable input acceptance.
pub fn load_steer_images(paths: &[String]) -> Result<NativeImageLoad, String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for raw in paths {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("图片路径不能为空".into());
        }
        if seen.insert(raw) {
            unique.push(raw);
        }
    }
    if unique.len() > MAX_NATIVE_IMAGES {
        return Err(format!("每条转向最多附带 {MAX_NATIVE_IMAGES} 张图片"));
    }
    let mut loaded = NativeImageLoad::default();
    for raw in unique {
        let path = Path::new(raw);
        if !is_allowed_media_path(path) {
            return Err(format!("不支持的附件类型：{raw}"));
        }
        let metadata =
            fs::metadata(path).map_err(|error| format!("无法读取图片 {raw}：{error}"))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(format!("图片不是有效的非空文件：{raw}"));
        }
        let limit = media_byte_limit(path);
        if metadata.len() > limit {
            return Err(format!("附件超过大小上限：{raw}"));
        }
        let file = fs::File::open(path).map_err(|error| format!("无法读取图片 {raw}：{error}"))?;
        let mut bytes = Vec::new();
        // Metadata may change between inspection and reading. Bound the actual
        // read as well, so a growing file cannot bypass the attachment limit.
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("读取附件失败 {raw}：{error}"))?;
        if bytes.is_empty() || bytes.len() as u64 > limit {
            return Err(format!("附件为空或超过大小上限：{raw}"));
        }
        if !media_bytes_match_extension(path, &bytes) {
            return Err(format!("附件内容与类型不符：{raw}"));
        }
        loaded.images.push(NativeImage {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| raw.to_string()),
            mime_type: media_mime(path).to_string(),
            data_base64: BASE64.encode(bytes),
            attachment_id: attachment_id_of(path),
            page: None,
            time_range: None,
        });
        loaded.loaded_paths.push(raw.to_string());
    }
    Ok(loaded)
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
    media_mime(path)
}

fn text_mime(extension: Option<&str>) -> Option<&'static str> {
    match extension? {
        "html" | "htm" => Some("text/html"),
        "json" => Some("application/json"),
        "xml" => Some("application/xml"),
        "txt" | "md" | "markdown" | "csv" | "css" | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx"
        | "py" | "rs" | "go" | "java" | "kt" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "rb"
        | "php" | "sh" | "bash" | "zsh" | "yml" | "yaml" | "toml" | "sql" | "log" | "vue"
        | "svelte" => Some("text/plain"),
        _ => None,
    }
}

fn media_mime(path: &Path) -> &'static str {
    let extension = image_extension(path);
    if let Some(mime) = crate::native::office_text::office_mime(extension.as_deref()) {
        return mime;
    }
    if let Some(mime) = text_mime(extension.as_deref()) {
        return mime;
    }
    match extension.as_deref() {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        _ => "image/png",
    }
}

fn media_byte_limit(path: &Path) -> u64 {
    let extension = image_extension(path);
    if crate::native::office_text::office_mime(extension.as_deref()).is_some() {
        return 32 * 1024 * 1024;
    }
    if text_mime(extension.as_deref()).is_some() {
        return 1024 * 1024;
    }
    match image_extension(path).as_deref() {
        Some("pdf") => 32 * 1024 * 1024,
        Some("mp4") => 6 * 1024 * 1024,
        _ => MAX_NATIVE_IMAGE_BYTES,
    }
}

fn attachment_id_of(path: &Path) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    let Some(objects) = parent.parent() else {
        return String::new();
    };
    if objects.file_name().and_then(|name| name.to_str()) != Some("objects") {
        return String::new();
    }
    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn media_bytes_match_extension(path: &Path, bytes: &[u8]) -> bool {
    match image_extension(path).as_deref() {
        Some("pdf") => bytes.starts_with(b"%PDF-"),
        Some("mp4") => bytes.len() >= 12 && &bytes[4..8] == b"ftyp",
        Some(ext) if crate::native::office_text::office_mime(Some(ext)).is_some() => {
            crate::native::office_text::office_bytes_match(Some(ext), bytes)
        }
        Some(ext) if text_mime(Some(ext)).is_some() => std::str::from_utf8(bytes).is_ok(),
        _ => true,
    }
}

fn image_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

fn is_allowed_media_path(path: &Path) -> bool {
    let extension = image_extension(path);
    text_mime(extension.as_deref()).is_some()
        || crate::native::office_text::office_mime(extension.as_deref()).is_some()
        || matches!(
            extension.as_deref(),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "pdf" | "mp4")
        )
}

fn is_allowed_image_path(path: &Path) -> bool {
    is_allowed_media_path(path)
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
    let limit = media_byte_limit(Path::new(name));
    if bytes.len() as u64 > limit {
        return Err(format!(
            "{}（超过大小上限）",
            sanitize_attachment_name(name)
        ));
    }
    if text_mime(image_extension(Path::new(name)).as_deref()).is_some()
        && std::str::from_utf8(bytes).is_err()
    {
        return Err(format!(
            "{} 不是 UTF-8 文本",
            sanitize_attachment_name(name)
        ));
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
        return Err(format!("附件不存在: {}", source.display()));
    }
    if !is_allowed_image_path(source) {
        return Err(format!("不支持的附件类型: {}", source.display()));
    }
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image.png".to_string());
    let bytes = fs::read(source).map_err(|error| format!("读取附件失败: {error}"))?;
    stage_image_bytes(app_config_dir, &name, &bytes)
}

pub async fn remember_loaded_media(
    pool: &sqlx::SqlitePool,
    root: &Path,
    loaded: &mut NativeImageLoad,
) -> Result<(), String> {
    let service = crate::native::attachments::AttachmentService::new(
        root.to_path_buf(),
        pool.clone(),
        "composer",
    );
    for image in &mut loaded.images {
        if !image.attachment_id.is_empty() {
            continue;
        }
        let bytes = BASE64
            .decode(image.data_base64.trim())
            .map_err(|_| format!("附件数据无效: {}", image.name))?;
        let stored = service
            .import_bytes(&image.name, &bytes, "composer")
            .await
            .map_err(|error| error.to_string())?;
        image.attachment_id = stored.id;
    }
    Ok(())
}

pub async fn hydrate_message_media(
    pool: &sqlx::SqlitePool,
    root: &Path,
    messages: &mut [crate::native::model::types::Message],
) -> Result<(), String> {
    let service = crate::native::attachments::AttachmentService::new(
        root.to_path_buf(),
        pool.clone(),
        "composer",
    );
    for message in messages {
        if message.media.is_empty() || !message.images.is_empty() {
            continue;
        }
        for item in &message.media {
            let described = match service.describe(item.attachment_id()).await {
                Ok(described) => described,
                Err(error) => {
                    message
                        .content
                        .push_str(&format!("\n[附件缺失] {} {error}", item.attachment_id()));
                    continue;
                }
            };
            let bytes = match service.read_bytes(item.attachment_id()).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    message.content.push_str(&format!(
                        "\n[附件缺失] {} {} {error}",
                        described.id, described.original_name
                    ));
                    continue;
                }
            };
            message.images.push(NativeImage {
                name: described.original_name,
                mime_type: described.mime,
                data_base64: BASE64.encode(bytes),
                attachment_id: described.id,
                page: None,
                time_range: None,
            });
        }
    }
    Ok(())
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
        .map_err(|_| "附件数据无效".to_string())?;
    let path = stage_image_bytes(&dir, &name, &bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn extract_attachment_text(
    name: String,
    data_base64: String,
) -> Result<crate::native::office_text::OfficePreview, String> {
    if data_base64.len() > 32 * 1024 * 1024 * 4 / 3 + 16 {
        return Err("附件超过大小上限".to_string());
    }
    let bytes = BASE64
        .decode(data_base64.trim())
        .map_err(|_| "附件数据无效".to_string())?;
    if bytes.len() as u64 > 32 * 1024 * 1024 {
        return Err("附件超过大小上限".to_string());
    }
    crate::native::office_text::preview_office(&name, &bytes)
}

#[tauri::command]
pub fn extract_staged_attachment_text(
    app: AppHandle,
    path: String,
) -> Result<crate::native::office_text::OfficePreview, String> {
    let root = attachments_dir(&app_config_dir(&app)?)
        .canonicalize()
        .map_err(|error| format!("无法读取附件目录: {error}"))?;
    let candidate = Path::new(path.trim())
        .canonicalize()
        .map_err(|_| "附件不存在".to_string())?;
    if !candidate.starts_with(&root) {
        return Err("只能预览已添加的附件".to_string());
    }
    let bytes = fs::read(&candidate).map_err(|error| format!("读取附件失败: {error}"))?;
    let name = candidate
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    crate::native::office_text::preview_office(&name, &bytes)
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

    #[test]
    fn steer_rejects_missing_or_unsupported_attachments_without_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let valid = stage_image_bytes(root.path(), "valid.png", b"\x89PNG\r\n").unwrap();
        for other in [
            root.path().join("missing.png"),
            root.path().join("document.pdf"),
        ] {
            if other.extension().unwrap() == "pdf" {
                fs::write(&other, b"document").unwrap();
            }
            assert!(load_steer_images(&[
                valid.to_string_lossy().into_owned(),
                other.to_string_lossy().into_owned(),
            ])
            .is_err());
            assert!(valid.exists(), "a rejected input must retain staged images");
        }
    }

    #[test]
    fn steer_rejects_empty_oversized_and_excess_attachments() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.png");
        let file = fs::File::create(&path).unwrap();
        let input = [path.to_string_lossy().into_owned()];
        assert!(load_steer_images(&input).is_err());
        file.set_len(MAX_NATIVE_IMAGE_BYTES + 1).unwrap();
        assert!(load_steer_images(&input).is_err());
        let inputs: Vec<String> = (0..=MAX_NATIVE_IMAGES)
            .map(|index| {
                let path = root.path().join(format!("{index}.png"));
                fs::write(&path, b"\x89PNG\r\n").unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        assert!(load_steer_images(&inputs).is_err());
    }

    #[test]
    fn steer_loads_supported_unique_images_without_removing_staging() {
        let root = tempfile::tempdir().unwrap();
        let path = stage_image_bytes(root.path(), "valid.PNG", b"\x89PNG\r\n").unwrap();
        let raw = path.to_string_lossy().into_owned();
        let loaded = load_steer_images(&[raw.clone(), raw.clone()]).unwrap();
        assert_eq!(loaded.images.len(), 1);
        assert_eq!(loaded.loaded_paths, vec![raw]);
        assert_eq!(loaded.images[0].mime_type, "image/png");
        assert!(loaded.skipped.is_empty() && loaded.missing.is_empty());
        assert!(path.exists());
    }

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
    fn stages_utf8_html_and_skips_other_files() {
        let root = tempfile::tempdir().unwrap();
        let html = root.path().join("qwen3.8-27b-Test2.html");
        fs::write(&html, "<h1>你好</h1>").unwrap();
        let staged = stage_image_from_path(root.path(), &html).unwrap();
        let loaded = load_native_images(Some(&[staged.to_string_lossy().into_owned()]));
        assert_eq!(loaded.images.len(), 1);
        assert_eq!(loaded.images[0].mime_type, "text/html");
        assert!(loaded.skipped.is_empty());
        let bin = root.path().join("tool.bin");
        fs::write(&bin, [0, 1]).unwrap();
        let skipped = load_native_images(Some(&[bin.to_string_lossy().into_owned()]));
        assert!(skipped.images.is_empty());
        assert!(skipped.skipped[0].contains("不支持的附件类型"));
        assert!(stage_image_bytes(root.path(), "bad.html", &[0xff, 0xfe]).is_err());
    }

    #[test]
    fn data_url_uses_mime_and_payload() {
        let image = NativeImage {
            name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "QQ==".to_string(),
            attachment_id: String::new(),
            page: None,
            time_range: None,
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
        assert!(stage_image_from_path(&root, &text).is_ok());

        let other = root.join("payload.exe");
        fs::write(&other, b"MZ").expect("other");
        assert!(stage_image_from_path(&root, &other).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
