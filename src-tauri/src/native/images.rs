#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use crate::native::model::types::NativeImage;

pub const MAX_NATIVE_IMAGES: usize = 8;
pub const MAX_NATIVE_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct NativeImageLoad {
    pub images: Vec<NativeImage>,
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
            Ok(bytes) => loaded.images.push(NativeImage {
                name,
                mime_type: image_mime_type(path).to_string(),
                data_base64: BASE64.encode(bytes),
            }),
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
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    }
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
}
