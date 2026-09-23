//! 请求投影、预算和图片变体。字节上限含等号。

use std::io::Cursor;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::ImageDecoder;
use serde_json::{json, Value};

use crate::native::media_error::{
    MediaError, MEDIA_BUDGET_EXCEEDED, VIDEO_INVALID, VIDEO_UNSUPPORTED,
};
use crate::native::media_limits::{
    BudgetLimits, MAX_DECODE_PIXELS, MAX_IMAGE_EDGE, MAX_PROCESSED_IMAGE_BYTES, MAX_REQUEST_BYTES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaWire {
    OpenaiChat,
    Anthropic,
    Responses,
    Kimi,
    Doubao,
    Qwen,
    Gemini,
}

pub(crate) fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

pub(crate) fn ensure_request_within_limit(
    value: &Value,
    limit: usize,
) -> Result<usize, MediaError> {
    let len = serialized_len(value);
    if len > limit {
        return Err(MediaError::new(
            MEDIA_BUDGET_EXCEEDED,
            format!("请求 {len} 字节，上限 {limit}"),
        ));
    }
    Ok(len)
}

pub(crate) fn base64_wire_len(raw_len: usize) -> usize {
    4 * raw_len.div_ceil(3)
}

pub(crate) fn video_content_part(
    wire: MediaWire,
    filename_mime: &str,
    raw: &[u8],
) -> Result<Value, MediaError> {
    let encoded = BASE64.encode(raw);
    let data_url = format!("data:{filename_mime};base64,{encoded}");
    match wire {
        MediaWire::Kimi | MediaWire::Doubao => Ok(json!({
            "type": "video_url",
            "video_url": {"url": data_url}
        })),
        MediaWire::Qwen => Ok(json!({
            "type": "video_url",
            "video_url": {"url": data_url}
        })),
        MediaWire::Gemini => Ok(json!({
            "inlineData": {"mimeType": filename_mime, "data": encoded}
        })),
        MediaWire::OpenaiChat | MediaWire::Anthropic | MediaWire::Responses => {
            Err(MediaError::new(VIDEO_UNSUPPORTED, "当前协议不能内联视频"))
        }
    }
}

pub(crate) fn pdf_content_part(
    wire: MediaWire,
    filename: &str,
    raw: &[u8],
) -> Result<Value, MediaError> {
    let encoded = BASE64.encode(raw);
    let data_url = format!("data:application/pdf;base64,{encoded}");
    match wire {
        MediaWire::OpenaiChat => Ok(json!({
            "type": "file",
            "file": {"filename": filename, "file_data": data_url}
        })),
        MediaWire::Anthropic => Ok(json!({
            "type": "document",
            "source": {"type": "base64", "media_type": "application/pdf", "data": encoded}
        })),
        MediaWire::Responses => Ok(json!({
            "type": "input_file",
            "filename": filename,
            "file_data": data_url
        })),
        MediaWire::Gemini => Ok(json!({
            "inlineData": {"mimeType": "application/pdf", "data": encoded}
        })),
        MediaWire::Kimi | MediaWire::Doubao | MediaWire::Qwen => Err(MediaError::new(
            VIDEO_INVALID,
            "该厂商原生路径不直接发送 PDF 原件",
        )),
    }
}

pub(crate) fn check_count_limit(actual: u64, limit: u64, label: &str) -> Result<(), MediaError> {
    if actual > limit {
        return Err(MediaError::new(
            MEDIA_BUDGET_EXCEEDED,
            format!("{label} 实际 {actual}，上限 {limit}"),
        ));
    }
    Ok(())
}

pub(crate) fn production_request_limit() -> usize {
    MAX_REQUEST_BYTES
}

pub(crate) fn limits_or_stricter(channel_original: Option<u64>, limits: BudgetLimits) -> u64 {
    channel_original
        .filter(|limit| *limit > 0)
        .map(|limit| limit.min(limits.max_original_bytes))
        .unwrap_or(limits.max_original_bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub staticized_animation: bool,
}

pub(crate) fn prepare_image(
    bytes: &[u8],
    limits: &BudgetLimits,
) -> Result<PreparedImage, MediaError> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| MediaError::new(MEDIA_BUDGET_EXCEEDED, error.to_string()))?;
    let format = reader.format();
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| MediaError::new(MEDIA_BUDGET_EXCEEDED, error.to_string()))?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels > limits.max_decode_pixels {
        return Err(MediaError::new(
            MEDIA_BUDGET_EXCEEDED,
            format!("解码像素 {pixels} 超过 {}", limits.max_decode_pixels),
        ));
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| MediaError::new(MEDIA_BUDGET_EXCEEDED, error.to_string()))?;
    image.apply_orientation(orientation);
    let staticized = format == Some(image::ImageFormat::Gif) && gif_frames(bytes) > 1;
    image = fit_edge(image, limits.max_image_edge);
    let (encoded, mime) = encode_within(&image, limits.max_processed_image_bytes)?;
    let (width, height) = (image.width(), image.height());
    Ok(PreparedImage {
        bytes: encoded,
        mime,
        width,
        height,
        staticized_animation: staticized,
    })
}

fn fit_edge(image: image::DynamicImage, max_edge: u32) -> image::DynamicImage {
    let edge = image.width().max(image.height());
    if edge <= max_edge || max_edge == 0 {
        return image;
    }
    let scale = f64::from(max_edge) / f64::from(edge);
    let width = ((f64::from(image.width()) * scale).round() as u32).max(1);
    let height = ((f64::from(image.height()) * scale).round() as u32).max(1);
    image.resize(width, height, image::imageops::FilterType::Triangle)
}

fn encode_within(
    image: &image::DynamicImage,
    max_bytes: usize,
) -> Result<(Vec<u8>, String), MediaError> {
    if image.color().has_alpha() {
        let bytes = encode_png(image);
        if bytes.len() <= max_bytes {
            return Ok((bytes, "image/png".to_string()));
        }
        return Err(MediaError::new(
            MEDIA_BUDGET_EXCEEDED,
            "透明图片超过处理后上限",
        ));
    }
    let mut quality = 85u8;
    loop {
        let bytes = encode_jpeg(image, quality);
        if bytes.len() <= max_bytes || quality <= 40 {
            if bytes.len() > max_bytes {
                return Err(MediaError::new(MEDIA_BUDGET_EXCEEDED, "图片超过处理后上限"));
            }
            return Ok((bytes, "image/jpeg".to_string()));
        }
        quality -= 15;
    }
}

fn encode_png(image: &image::DynamicImage) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("png encode");
    cursor.into_inner()
}

fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
    image.write_with_encoder(encoder).expect("jpeg encode");
    cursor.into_inner()
}

fn gif_frames(bytes: &[u8]) -> u32 {
    bytes.iter().filter(|byte| **byte == 0x2c).count() as u32
}

pub(crate) fn decode_pixel_limit() -> u64 {
    MAX_DECODE_PIXELS
}

pub(crate) fn processed_image_limit() -> usize {
    MAX_PROCESSED_IMAGE_BYTES
}

pub(crate) fn default_edge() -> u32 {
    MAX_IMAGE_EDGE
}

pub(crate) fn media_wire_for_model(model: &str) -> MediaWire {
    match crate::native::model_catalog::lookup_catalog(model).map(|entry| entry.vendor.as_str()) {
        Some("kimi") => MediaWire::Kimi,
        Some("doubao") => MediaWire::Doubao,
        Some("qwen") => MediaWire::Qwen,
        Some("gemini") => MediaWire::Gemini,
        _ => MediaWire::OpenaiChat,
    }
}

pub(crate) fn model_allows_video(model: &str) -> bool {
    let Some(entry) = crate::native::model_catalog::lookup_catalog(model) else {
        return false;
    };
    entry.input_types.iter().any(|kind| kind == "video")
        && matches!(
            media_wire_for_model(model),
            MediaWire::Kimi | MediaWire::Doubao | MediaWire::Qwen | MediaWire::Gemini
        )
}

pub(crate) fn budget_for_model(model: &str) -> BudgetLimits {
    let mut limits = BudgetLimits::production();
    let Some(entry) = crate::native::model_catalog::lookup_catalog(model) else {
        return limits;
    };
    if !entry.input_types.iter().any(|kind| kind == "image") {
        limits.max_image_blocks = 0;
    }
    if !entry.input_types.iter().any(|kind| kind == "video") {
        limits.max_videos = 0;
        limits.max_video_bytes = 0;
    }
    limits
}

pub(crate) fn mp4_time_range(bytes: &[u8]) -> Option<String> {
    let index = bytes.windows(4).position(|mark| mark == b"mvhd")?;
    let payload = bytes.get(index + 4..)?;
    let version = *payload.first()?;
    let (timescale, duration) = if version == 1 {
        let timescale = u32::from_be_bytes(payload.get(4 + 16..4 + 20)?.try_into().ok()?);
        let duration = u64::from_be_bytes(payload.get(4 + 20..4 + 28)?.try_into().ok()?);
        (timescale, duration)
    } else {
        let timescale = u32::from_be_bytes(payload.get(12..16)?.try_into().ok()?);
        let duration = u32::from_be_bytes(payload.get(16..20)?.try_into().ok()?) as u64;
        (timescale, duration)
    };
    if timescale == 0 {
        return None;
    }
    let seconds = duration as f64 / f64::from(timescale);
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(format!("0-{seconds:.1}s"))
}

pub(crate) fn shrink_message_images(
    messages: &mut [crate::native::model::types::Message],
    limits: &BudgetLimits,
) -> Vec<String> {
    use crate::native::model::types::NativeImage;
    let mut notices = Vec::new();
    for message in messages.iter_mut() {
        let mut kept = Vec::with_capacity(message.images.len());
        let mut dropped = Vec::new();
        let mut images_kept = 0usize;
        let mut videos_kept = 0usize;
        for image in std::mem::take(&mut message.images) {
            if image.mime_type.starts_with("video/") {
                if limits.max_videos == 0 || videos_kept >= limits.max_videos {
                    dropped.push(image);
                    continue;
                }
                videos_kept += 1;
                kept.push(image);
                continue;
            }
            if !image.mime_type.starts_with("image/") {
                kept.push(image);
                continue;
            }
            if limits.max_image_blocks == 0 || images_kept >= limits.max_image_blocks {
                dropped.push(image);
                continue;
            }
            let Ok(raw) = BASE64.decode(image.data_base64.trim()) else {
                dropped.push(image);
                continue;
            };
            if !image_needs_shrink(&raw, limits) {
                images_kept += 1;
                kept.push(image);
                continue;
            }
            match prepare_image(&raw, limits) {
                Ok(prepared) => {
                    notices.push(format!("图片已按预算压缩：{}", image.name));
                    images_kept += 1;
                    kept.push(NativeImage {
                        name: image.name,
                        mime_type: prepared.mime,
                        data_base64: BASE64.encode(prepared.bytes),
                        attachment_id: image.attachment_id,
                        page: image.page,
                        time_range: image.time_range,
                    });
                }
                Err(_) => dropped.push(image),
            }
        }
        if !dropped.is_empty() {
            let summary = image_ref_summary(&dropped);
            message.content.push_str(&format!("\n[媒体降级] {summary}"));
            notices.push(format!("图片超过预算，已改为引用：{summary}"));
        }
        message.images = kept;
    }
    notices
}

fn image_needs_shrink(bytes: &[u8], limits: &BudgetLimits) -> bool {
    if bytes.len() > limits.max_processed_image_bytes {
        return true;
    }
    let Ok(reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format() else {
        return false;
    };
    let Ok((width, height)) = reader.into_dimensions() else {
        return false;
    };
    width > limits.max_image_edge
        || height > limits.max_image_edge
        || u64::from(width) * u64::from(height) > limits.max_decode_pixels
}

fn image_ref_summary(images: &[crate::native::model::types::NativeImage]) -> String {
    images
        .iter()
        .map(|image| {
            let id = if image.attachment_id.is_empty() {
                "未编号"
            } else {
                image.attachment_id.as_str()
            };
            let mut line = format!("附件 {id} 名称 {}", image.name);
            if let Some(page) = image.page {
                line.push_str(&format!(" 第 {page} 页"));
            }
            if let Some(range) = image.time_range.as_deref() {
                line.push_str(&format!(" 时间段 {range}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("；")
}

pub(crate) fn degrade_history_media(
    mut messages: Vec<crate::native::model::types::Message>,
    max_media_bytes: usize,
) -> (Vec<crate::native::model::types::Message>, Vec<String>) {
    use crate::native::model::types::Role;
    let media_bytes = |items: &[crate::native::model::types::Message]| {
        items
            .iter()
            .flat_map(|message| message.images.iter())
            .map(|image| image.data_base64.len())
            .sum::<usize>()
    };
    if media_bytes(&messages) <= max_media_bytes {
        return (messages, Vec::new());
    }
    let protected = messages
        .iter()
        .rposition(|message| message.role == Role::User);
    let mut notices = Vec::new();
    for index in 0..messages.len() {
        if Some(index) == protected || messages[index].images.is_empty() {
            continue;
        }
        let summary = image_ref_summary(&messages[index].images);
        messages[index].images.clear();
        messages[index]
            .content
            .push_str(&format!("\n[媒体降级] {summary}"));
        notices.push(format!("历史媒体已降级为引用：{summary}"));
        if media_bytes(&messages) <= max_media_bytes {
            break;
        }
    }
    (messages, notices)
}

pub(crate) fn preflight_message_media(
    protocol: &str,
    model: &str,
    messages: &[crate::native::model::types::Message],
) -> Result<(), String> {
    for message in messages {
        for image in &message.images {
            if image.mime_type.starts_with("video/") && !video_allowed(protocol, model) {
                return Err("video_unsupported: 当前模型不能接收视频".to_string());
            }
        }
    }
    Ok(())
}

fn video_allowed(protocol: &str, model: &str) -> bool {
    protocol == crate::native::protocol::PROTOCOL_OPENAI && model_allows_video(model)
}

pub(crate) fn binary_part(
    protocol: &str,
    model: &str,
    name: &str,
    mime: &str,
    data_base64: &str,
) -> Result<Value, String> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    let raw = BASE64
        .decode(data_base64.trim())
        .map_err(|_| "invalid_chunk: 附件数据不是合法 base64".to_string())?;
    if crate::native::office_text::office_mime(Some(extension_of_name(name))).is_some()
        || is_office_attachment(mime)
    {
        let text = crate::native::office_text::extract_office_text(name, &raw)
            .map_err(|error| format!("import_invalid: {error}"))?;
        return Ok(text_attachment_part(protocol, name, &text));
    }
    if is_text_attachment(mime) {
        let text = String::from_utf8(raw)
            .map_err(|_| format!("import_invalid: {name} 不是 UTF-8 文本"))?;
        return Ok(text_attachment_part(protocol, name, &text));
    }
    if mime.starts_with("video/") {
        if !video_allowed(protocol, model) {
            return Err("video_unsupported: 当前模型不能接收视频".to_string());
        }
        let wire = match media_wire_for_model(model) {
            MediaWire::Gemini => MediaWire::Kimi,
            other => other,
        };
        return video_content_part(wire, mime, &raw).map_err(|error| error.to_string());
    }
    if mime == "application/pdf" {
        let wire = media_wire_for_model(model);
        if matches!(wire, MediaWire::Kimi | MediaWire::Doubao | MediaWire::Qwen) {
            let report = crate::native::pdf_doc::read_pdf_text(&raw, None)
                .map_err(|error| error.to_string())?;
            return Ok(json!({"type": "text", "text": report.text}));
        }
        let part_wire = match protocol {
            "anthropic" => MediaWire::Anthropic,
            "codex" => MediaWire::Responses,
            _ => MediaWire::OpenaiChat,
        };
        return pdf_content_part(part_wire, name, &raw).map_err(|error| error.to_string());
    }
    Err(format!("import_invalid: 不支持的附件类型 {mime}"))
}

fn extension_of_name(name: &str) -> &str {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .rsplit('.')
        .next()
        .unwrap_or("")
}

fn is_office_attachment(mime: &str) -> bool {
    matches!(
        mime,
        "application/vnd.ms-excel"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/msword"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    )
}

fn is_text_attachment(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json" || mime == "application/xml"
}

fn text_attachment_part(protocol: &str, name: &str, text: &str) -> Value {
    let body = format!("[附件 {name}]\n{text}");
    match protocol {
        "anthropic" => json!({"type": "text", "text": body}),
        "codex" => json!({"type": "input_text", "text": body}),
        _ => json!({"type": "text", "text": body}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    fn exact_json(len: usize) -> Value {
        let overhead = serialized_len(&json!({"t": ""}));
        json!({"t": "a".repeat(len - overhead)})
    }

    #[test]
    fn tc_bud_001_request_limit_uses_serialized_bytes() {
        let limit = production_request_limit();
        assert_eq!(limit, 16_777_216);
        assert!(ensure_request_within_limit(&exact_json(limit - 1), limit).is_ok());
        assert!(ensure_request_within_limit(&exact_json(limit), limit).is_ok());
        assert_eq!(
            ensure_request_within_limit(&exact_json(limit + 1), limit)
                .unwrap_err()
                .code,
            MEDIA_BUDGET_EXCEEDED
        );
    }

    #[test]
    fn tc_bud_002_equal_to_limit_is_accepted() {
        assert!(check_count_limit(8, 8, "附件").is_ok());
        assert_eq!(
            check_count_limit(9, 8, "附件").unwrap_err().code,
            MEDIA_BUDGET_EXCEEDED
        );
        assert_eq!(
            limits_or_stricter(Some(1024), BudgetLimits::production()),
            1024
        );
    }

    #[test]
    fn tc_vid_001_native_video_parts_round_trip_bytes() {
        let raw = b"video-bytes";
        for wire in [MediaWire::Kimi, MediaWire::Doubao, MediaWire::Qwen] {
            let part = video_content_part(wire, "video/mp4", raw).unwrap();
            assert_eq!(part["type"], "video_url");
            let url = part["video_url"]["url"].as_str().unwrap();
            let encoded = url.split_once(";base64,").unwrap().1;
            assert_eq!(BASE64.decode(encoded).unwrap(), raw);
        }
        let gemini = video_content_part(MediaWire::Gemini, "video/mp4", raw).unwrap();
        assert_eq!(
            BASE64
                .decode(gemini["inlineData"]["data"].as_str().unwrap())
                .unwrap(),
            raw
        );
        assert_eq!(
            video_content_part(MediaWire::OpenaiChat, "video/mp4", raw)
                .unwrap_err()
                .code,
            VIDEO_UNSUPPORTED
        );
    }

    #[test]
    fn tc_model_001_pdf_parts_use_protocol_fields() {
        let raw = b"%PDF-1.4";
        let openai = pdf_content_part(MediaWire::OpenaiChat, "a.pdf", raw).unwrap();
        assert_eq!(openai["type"], "file");
        assert_eq!(openai["file"]["filename"], "a.pdf");
        let anthropic = pdf_content_part(MediaWire::Anthropic, "a.pdf", raw).unwrap();
        assert_eq!(anthropic["type"], "document");
        assert_eq!(anthropic["source"]["media_type"], "application/pdf");
        let responses = pdf_content_part(MediaWire::Responses, "a.pdf", raw).unwrap();
        assert_eq!(responses["type"], "input_file");
    }

    #[test]
    fn tc_bud_003_small_png_prepares_without_dropping_pixels() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([9, 8, 7]),
        ));
        let mut cursor = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        let prepared = prepare_image(&cursor.into_inner(), &BudgetLimits::production()).unwrap();
        assert_eq!((prepared.width, prepared.height), (2, 2));
        assert!(!prepared.staticized_animation);
        assert!(prepared.bytes.len() <= processed_image_limit());
        let _ = (decode_pixel_limit(), default_edge());
    }

    #[test]
    fn tc_bud_007_history_media_degrades_to_refs_and_keeps_current_input() {
        use crate::native::model::types::{Message, NativeImage};
        let old = {
            let mut message = Message::user("earlier");
            message.images.push(NativeImage {
                name: "old.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                data_base64: "AAAA".repeat(20),
                attachment_id: "att-old".to_string(),
                page: Some(3),
                time_range: None,
            });
            message
        };
        let current = {
            let mut message = Message::user("now");
            message.images.push(NativeImage {
                name: "now.png".to_string(),
                mime_type: "image/png".to_string(),
                data_base64: "QQ==".to_string(),
                attachment_id: "att-now".to_string(),
                page: None,
                time_range: None,
            });
            message
        };
        let (messages, notices) = degrade_history_media(vec![old, current], 8);
        assert!(notices[0].contains("att-old"));
        assert!(messages[0].images.is_empty());
        assert!(messages[0].content.contains("att-old"));
        assert!(messages[0].content.contains("old.pdf"));
        assert!(messages[0].content.contains("第 3 页"));
        assert!(!messages[0].content.contains("AAAA"));
        assert_eq!(messages[1].images.len(), 1);
        assert_eq!(messages[1].images[0].attachment_id, "att-now");
    }

    #[test]
    fn tc_bud_008_wide_image_is_compressed_and_small_image_is_kept() {
        use crate::native::model::types::{Message, NativeImage};
        let wide = image::RgbImage::from_fn(3000, 8, |x, _| image::Rgb([(x % 200) as u8, 20, 40]));
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(wide)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        let encoded = BASE64.encode(cursor.into_inner());
        let mut wide_message = Message::user("wide");
        wide_message.images.push(NativeImage {
            name: "wide.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: encoded.clone(),
            attachment_id: "att-wide".to_string(),
            page: None,
            time_range: None,
        });
        let tiny = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([1, 2, 3]),
        ));
        let mut tiny_cursor = std::io::Cursor::new(Vec::new());
        tiny.write_to(&mut tiny_cursor, image::ImageFormat::Png)
            .unwrap();
        let tiny_encoded = BASE64.encode(tiny_cursor.into_inner());
        let mut small = Message::user("small");
        small.images.push(NativeImage {
            name: "small.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: tiny_encoded.clone(),
            attachment_id: "att-small".to_string(),
            page: None,
            time_range: None,
        });
        let mut messages = vec![wide_message, small];
        let notices = shrink_message_images(&mut messages, &BudgetLimits::production());
        assert!(notices.iter().any(|notice| notice.contains("wide.png")));
        assert_ne!(messages[0].images[0].data_base64, encoded);
        assert_eq!(messages[0].images[0].attachment_id, "att-wide");
        let shrunk = BASE64
            .decode(messages[0].images[0].data_base64.trim())
            .unwrap();
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(shrunk))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert!(width <= 2048);
        assert!(height <= 2048);
        assert!(!messages[0].content.contains(&encoded));
        assert_eq!(messages[1].images[0].data_base64, tiny_encoded);
        assert!(notices.iter().all(|notice| !notice.contains("small.png")));
    }

    #[test]
    fn tc_bud_009_model_budget_drops_unsupported_media_and_keeps_refs() {
        use crate::native::model::types::{Message, NativeImage};
        let text_only = budget_for_model("deepseek-chat");
        assert_eq!(text_only.max_image_blocks, 0);
        assert_eq!(text_only.max_videos, 0);
        let vision = budget_for_model("gpt-4o");
        assert!(vision.max_image_blocks > 0);
        assert_eq!(vision.max_videos, 0);
        let mut history = Message::user("earlier");
        history.images.push(NativeImage {
            name: "page-2.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "AAAA".repeat(30),
            attachment_id: "att-page".to_string(),
            page: Some(2),
            time_range: None,
        });
        history.images.push(NativeImage {
            name: "clip.mp4".to_string(),
            mime_type: "video/mp4".to_string(),
            data_base64: "BBBB".repeat(30),
            attachment_id: "att-clip".to_string(),
            page: None,
            time_range: Some("0-3.5s".to_string()),
        });
        let mut current = Message::user("now");
        current.images.push(NativeImage {
            name: "now.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "QQ==".to_string(),
            attachment_id: "att-now".to_string(),
            page: None,
            time_range: None,
        });
        let mut messages = vec![history, current];
        let notices = shrink_message_images(&mut messages, &text_only);
        assert!(messages[0].images.is_empty());
        assert!(messages[0].content.contains("第 2 页"));
        assert!(messages[0].content.contains("时间段 0-3.5s"));
        assert!(messages[0].content.contains("page-2.png"));
        assert!(messages[0].content.contains("clip.mp4"));
        assert!(!messages[0].content.contains("AAAA"));
        assert!(!messages[0].content.contains("BBBB"));
        assert!(messages[1].images.is_empty());
        assert!(notices.iter().any(|notice| notice.contains("第 2 页")));
        let payload =
            b"mvhd\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x03\xe8\x00\x00\x0b\xb8";
        assert_eq!(mp4_time_range(payload).as_deref(), Some("0-3.0s"));
        let mut older = Message::user("older");
        older.images.push(NativeImage {
            name: "page-1.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "C".repeat(40),
            attachment_id: "att-1".to_string(),
            page: Some(1),
            time_range: None,
        });
        let mut middle = Message::user("middle");
        middle.images.push(NativeImage {
            name: "page-4.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "D".repeat(40),
            attachment_id: "att-4".to_string(),
            page: Some(4),
            time_range: None,
        });
        let mut latest = Message::user("latest");
        latest.images.push(NativeImage {
            name: "keep.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "E".repeat(8),
            attachment_id: "att-keep".to_string(),
            page: None,
            time_range: None,
        });
        let (degraded, degrade_notices) = degrade_history_media(vec![older, middle, latest], 50);
        assert!(degraded[0].images.is_empty());
        assert!(degraded[0].content.contains("第 1 页"));
        assert!(!degraded[0].content.contains(&"C".repeat(40)));
        assert_eq!(degraded[2].images.len(), 1);
        assert!(degraded[2].images[0].data_base64.contains('E'));
        assert!(degrade_notices
            .iter()
            .any(|notice| notice.contains("第 1 页")));
        let kept_bytes: usize = degraded
            .iter()
            .flat_map(|message| message.images.iter())
            .map(|image| image.data_base64.len())
            .sum();
        assert!(kept_bytes <= 50);
    }

    #[test]
    fn html_attachment_becomes_text_and_not_base64() {
        let encoded = BASE64.encode("<h1>你好</h1>");
        let part = binary_part(
            "openai",
            "glm-5.3-flash",
            "qwen3.8-27b-Test2.html",
            "text/html",
            &encoded,
        )
        .unwrap();
        assert_eq!(part["type"], "text");
        let text = part["text"].as_str().unwrap();
        assert!(text.contains("qwen3.8-27b-Test2.html"));
        assert!(text.contains("你好"));
        assert!(!text.contains(&encoded));
        let responses = binary_part("codex", "gpt-4o", "a.html", "text/html", &encoded).unwrap();
        assert_eq!(responses["type"], "input_text");
    }
}
