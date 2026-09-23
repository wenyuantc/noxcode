use std::io::Cursor;

use image::ImageDecoder;
use serde_json::json;

use crate::native::media_error::{MediaError, IMPORT_INVALID, PDF_INVALID, VIDEO_INVALID};
use crate::native::media_limits::MAX_DECODE_PIXELS;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sniffed {
    pub mime: String,
    pub media_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub page_count: Option<u32>,
    pub duration_seconds: Option<f64>,
    pub animated: bool,
    pub orientation: u8,
    pub pixel_count: Option<u64>,
}

impl Sniffed {
    pub(crate) fn metadata_json(&self) -> String {
        json!({
            "width": self.width,
            "height": self.height,
            "page_count": self.page_count,
            "duration_seconds": self.duration_seconds,
            "animated": self.animated,
            "orientation": self.orientation,
        })
        .to_string()
    }
}

pub(crate) fn inspect(
    bytes: &[u8],
    original_name: &str,
    declared_mime: &str,
) -> Result<Sniffed, MediaError> {
    if bytes.is_empty() {
        return Err(MediaError::new(IMPORT_INVALID, "空文件"));
    }
    let sniffed = if let Some(mime) =
        crate::native::office_text::office_mime(extension_of(original_name).as_deref())
    {
        if !crate::native::office_text::office_bytes_match(
            extension_of(original_name).as_deref(),
            bytes,
        ) {
            return Err(MediaError::new(IMPORT_INVALID, "扩展名与内容不符"));
        }
        Sniffed {
            mime: mime.to_string(),
            media_type: "document".to_string(),
            width: None,
            height: None,
            page_count: None,
            duration_seconds: None,
            animated: false,
            orientation: 1,
            pixel_count: None,
        }
    } else if let Some(mime) = text_mime_from_name(original_name) {
        if std::str::from_utf8(bytes).is_err() {
            return Err(MediaError::new(IMPORT_INVALID, "附件不是 UTF-8 文本"));
        }
        Sniffed {
            mime: mime.to_string(),
            media_type: "text".to_string(),
            width: None,
            height: None,
            page_count: None,
            duration_seconds: None,
            animated: false,
            orientation: 1,
            pixel_count: None,
        }
    } else if bytes.starts_with(b"%PDF-") {
        sniff_pdf(bytes)?
    } else if looks_like_mp4(bytes) {
        sniff_mp4(bytes)?
    } else {
        sniff_image(bytes)?
    };
    if let Some(expected) = mime_from_name(original_name) {
        if expected != sniffed.mime {
            return Err(MediaError::new(IMPORT_INVALID, "扩展名与内容不符"));
        }
    }
    let declared = declared_mime.trim();
    if !declared.is_empty() && declared != "application/octet-stream" && declared != sniffed.mime {
        return Err(MediaError::new(IMPORT_INVALID, "声明类型与内容不符"));
    }
    Ok(sniffed)
}

fn extension_of(name: &str) -> Option<String> {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = file_name.rsplit('.').next()?;
    if ext == file_name {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

fn text_mime_from_name(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "md" | "markdown" | "csv" | "css" | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx"
        | "py" | "rs" | "go" | "java" | "kt" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "rb"
        | "php" | "sh" | "bash" | "zsh" | "yml" | "yaml" | "toml" | "sql" | "log" | "vue"
        | "svelte" => "text/plain",
        _ => return None,
    })
}

fn mime_from_name(name: &str) -> Option<&'static str> {
    if let Some(mime) = text_mime_from_name(name) {
        return Some(mime);
    }
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        _ => return None,
    })
}

fn sniff_image(bytes: &[u8]) -> Result<Sniffed, MediaError> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
    let format = reader
        .format()
        .ok_or_else(|| MediaError::new(IMPORT_INVALID, "无法识别图片"))?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => return Err(MediaError::new(IMPORT_INVALID, "不支持的图片格式")),
    };
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_DECODE_PIXELS {
        return Err(MediaError::new(IMPORT_INVALID, "图片像素超过解码上限"));
    }
    let orientation = orientation_id(
        decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms),
    );
    let animated = format == image::ImageFormat::Gif && gif_frame_count(bytes) > 1;
    Ok(Sniffed {
        mime: mime.to_string(),
        media_type: "image".to_string(),
        width: Some(width),
        height: Some(height),
        page_count: None,
        duration_seconds: None,
        animated,
        orientation,
        pixel_count: Some(pixels),
    })
}

fn orientation_id(value: image::metadata::Orientation) -> u8 {
    match value {
        image::metadata::Orientation::NoTransforms => 1,
        image::metadata::Orientation::FlipHorizontal => 2,
        image::metadata::Orientation::Rotate180 => 3,
        image::metadata::Orientation::FlipVertical => 4,
        image::metadata::Orientation::Rotate90 => 6,
        image::metadata::Orientation::Rotate270 => 8,
        image::metadata::Orientation::Rotate90FlipH => 5,
        image::metadata::Orientation::Rotate270FlipH => 7,
    }
}

fn gif_frame_count(bytes: &[u8]) -> u32 {
    let mut count = 0u32;
    if bytes.len() < 13 || &bytes[..3] != b"GIF" {
        return 0;
    }
    let packed = bytes[10];
    let mut index = 13usize;
    if packed & 0x80 != 0 {
        let size = 3 * (1usize << ((packed & 0x07) + 1));
        index = index.saturating_add(size);
    }
    while index < bytes.len() {
        match bytes[index] {
            0x3b => break,
            0x21 => {
                index += 2;
                while index < bytes.len() {
                    let size = bytes[index] as usize;
                    index += 1 + size;
                    if size == 0 {
                        break;
                    }
                }
            }
            0x2c => {
                count += 1;
                if index + 10 > bytes.len() {
                    break;
                }
                let packed = bytes[index + 9];
                index += 10;
                if packed & 0x80 != 0 {
                    let size = 3 * (1usize << ((packed & 0x07) + 1));
                    index = index.saturating_add(size);
                }
                index += 1;
                while index < bytes.len() {
                    let size = bytes[index] as usize;
                    index += 1 + size;
                    if size == 0 {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    count
}

fn sniff_pdf(bytes: &[u8]) -> Result<Sniffed, MediaError> {
    if bytes.len() < 16 || !bytes.windows(9).any(|window| window == b"startxref") {
        return Err(MediaError::new(PDF_INVALID, "PDF 结构不完整"));
    }
    if !bytes.windows(5).any(|window| window == b"%%EOF") {
        return Err(MediaError::new(PDF_INVALID, "PDF 被截断"));
    }
    Ok(Sniffed {
        mime: "application/pdf".to_string(),
        media_type: "pdf".to_string(),
        width: None,
        height: None,
        page_count: pdf_page_count(bytes),
        duration_seconds: None,
        animated: false,
        orientation: 1,
        pixel_count: None,
    })
}

fn pdf_page_count(bytes: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(bytes);
    let mut count = 0u32;
    for line in text.split("/Type") {
        let trimmed = line.trim_start();
        if trimmed.starts_with("/Page") && !trimmed.starts_with("/Pages") {
            count += 1;
        }
    }
    (count > 0).then_some(count)
}

fn looks_like_mp4(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp"
}

fn sniff_mp4(bytes: &[u8]) -> Result<Sniffed, MediaError> {
    let info = parse_mp4(bytes).map_err(|error| MediaError::new(VIDEO_INVALID, error))?;
    if !info.has_video {
        return Err(MediaError::new(VIDEO_INVALID, "没有视频轨"));
    }
    Ok(Sniffed {
        mime: "video/mp4".to_string(),
        media_type: "video".to_string(),
        width: info.width,
        height: info.height,
        page_count: None,
        duration_seconds: info.duration_seconds,
        animated: false,
        orientation: 1,
        pixel_count: None,
    })
}

struct Mp4Info {
    has_video: bool,
    width: Option<u32>,
    height: Option<u32>,
    duration_seconds: Option<f64>,
}

fn parse_mp4(bytes: &[u8]) -> Result<Mp4Info, String> {
    let mut info = Mp4Info {
        has_video: false,
        width: None,
        height: None,
        duration_seconds: None,
    };
    walk_boxes(bytes, &mut info).map_err(|error| error.to_string())?;
    Ok(info)
}

fn walk_boxes(bytes: &[u8], info: &mut Mp4Info) -> Result<(), String> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let size32 = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let kind = std::str::from_utf8(&bytes[offset + 4..offset + 8]).unwrap_or("");
        let (header, size) = if size32 == 1 {
            if offset + 16 > bytes.len() {
                return Err("MP4 盒长度损坏".into());
            }
            let large = u64::from_be_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            (16usize, large as usize)
        } else if size32 == 0 {
            (8usize, bytes.len() - offset)
        } else {
            (8usize, size32 as usize)
        };
        if size < header || offset + size > bytes.len() {
            return Err("MP4 盒越界".into());
        }
        let body = &bytes[offset + header..offset + size];
        match kind {
            "moov" | "trak" | "mdia" | "minf" | "stbl" => walk_boxes(body, info)?,
            "mvhd" => {
                if let Some(seconds) = mvhd_seconds(body) {
                    info.duration_seconds = Some(seconds);
                }
            }
            "hdlr" => {
                if body.len() >= 12 && &body[8..12] == b"vide" {
                    info.has_video = true;
                }
            }
            "tkhd" => {
                if let Some((width, height)) = tkhd_size(body) {
                    info.width = Some(width);
                    info.height = Some(height);
                }
            }
            _ => {}
        }
        offset += size;
    }
    Ok(())
}

fn mvhd_seconds(body: &[u8]) -> Option<f64> {
    let version = *body.first()?;
    if version == 0 && body.len() >= 20 {
        let timescale = u32::from_be_bytes(body[12..16].try_into().ok()?) as f64;
        let duration = u32::from_be_bytes(body[16..20].try_into().ok()?) as f64;
        (timescale > 0.0).then_some(duration / timescale)
    } else if version == 1 && body.len() >= 32 {
        let timescale = u32::from_be_bytes(body[20..24].try_into().ok()?) as f64;
        let duration = u64::from_be_bytes(body[24..32].try_into().ok()?) as f64;
        (timescale > 0.0).then_some(duration / timescale)
    } else {
        None
    }
}

fn tkhd_size(body: &[u8]) -> Option<(u32, u32)> {
    let version = *body.first()?;
    let start = if version == 0 { 76 } else { 88 };
    if body.len() < start + 8 {
        return None;
    }
    let width = u32::from_be_bytes(body[start..start + 4].try_into().ok()?) >> 16;
    let height = u32::from_be_bytes(body[start + 4..start + 8].try_into().ok()?) >> 16;
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_text_is_stored_as_text() {
        let sniffed = inspect("<h1>你好</h1>".as_bytes(), "qwen3.8-27b-Test2.html", "").unwrap();
        assert_eq!(sniffed.mime, "text/html");
        assert_eq!(sniffed.media_type, "text");
        assert_eq!(
            inspect(b"\xff\xfe", "page.html", "").unwrap_err().code,
            IMPORT_INVALID
        );
    }
}
