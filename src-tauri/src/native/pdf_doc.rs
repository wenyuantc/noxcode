//! 本地 PDF 文本。页面图渲染仍依赖随包 Pdfium，缺失时只报告组件不可用。

use lopdf::Document;

use crate::native::media_error::{
    MediaError, CANCELLED, COMPONENT_MISSING, MEDIA_BUDGET_EXCEEDED, PDF_ENCRYPTED, PDF_INVALID,
    RANGE_REQUIRED, TIMEOUT,
};
use crate::native::media_limits::{MAX_NATIVE_PDF_PAGES, MAX_PDF_RENDER_PAGES, TEXT_BYTE_BUDGET};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PdfText {
    pub page_count: u32,
    pub pages: Vec<u32>,
    pub text: String,
    pub empty_pages: Vec<u32>,
    pub truncated: bool,
}

pub(crate) fn read_pdf_text(bytes: &[u8], pages: Option<&[u32]>) -> Result<PdfText, MediaError> {
    if bytes.is_empty() || !bytes.starts_with(b"%PDF-") {
        return Err(MediaError::new(PDF_INVALID, "不是有效 PDF"));
    }
    let document = Document::load_mem(bytes)
        .map_err(|error| MediaError::new(PDF_INVALID, error.to_string()))?;
    if document.is_encrypted() {
        return Err(MediaError::new(PDF_ENCRYPTED, "PDF 已加密"));
    }
    let found = document.get_pages();
    let page_count = found.len() as u32;
    if page_count == 0 {
        return Err(MediaError::new(PDF_INVALID, "PDF 没有页面"));
    }
    let selected = select_pages(page_count, pages)?;
    let mut chunks = Vec::new();
    let mut empty_pages = Vec::new();
    for page in &selected {
        let extracted = document
            .extract_text(&[*page])
            .map_err(|error| MediaError::new(PDF_INVALID, error.to_string()))?;
        let trimmed = extracted.trim();
        if trimmed.is_empty() {
            empty_pages.push(*page);
            chunks.push(format!("--- page {page} ---\n（此页没有文本）"));
        } else {
            chunks.push(format!("--- page {page} ---\n{trimmed}"));
        }
    }
    if empty_pages.len() == selected.len() {
        return Err(MediaError::new(
            PDF_INVALID,
            "所选页面没有文本层，不能当作已完整读取",
        ));
    }
    let mut text = chunks.join("\n");
    let mut truncated = false;
    if text.len() > TEXT_BYTE_BUDGET {
        let mut end = TEXT_BYTE_BUDGET;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n（文本已截断，未完整读取）");
        truncated = true;
    }
    Ok(PdfText {
        page_count,
        pages: selected,
        text,
        empty_pages,
        truncated,
    })
}

fn pdfium_missing() -> MediaError {
    MediaError::new(
        COMPONENT_MISSING,
        "PDF 页面图需要随包 Pdfium。请把对应平台的库放到应用资源目录 pdfium/ 后重试；没有该库时应用仍可启动，文本提取不受影响",
    )
}

pub(crate) fn render_pdf_pages() -> Result<(), MediaError> {
    crate::native::components::pdfium_library_path().map(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageImage {
    pub page: u32,
    pub png: Vec<u8>,
}

/// 校验页码后渲染页面图。没有随包 Pdfium 时返回 `component_missing`，不生成占位图。
/// 取消、超时、越界和组件缺失都会清掉临时目录。没有文本层的扫描页仍然渲染。
pub(crate) fn render_page_images(
    bytes: &[u8],
    pages: Option<&[u32]>,
    temp_dir: &std::path::Path,
    cancelled: bool,
    timed_out: bool,
) -> Result<Vec<PageImage>, MediaError> {
    render_page_images_with(
        bytes,
        pages,
        temp_dir,
        cancelled,
        timed_out,
        crate::native::components::pdfium_library_path().ok(),
    )
}

pub(crate) fn render_page_images_with(
    bytes: &[u8],
    pages: Option<&[u32]>,
    temp_dir: &std::path::Path,
    cancelled: bool,
    timed_out: bool,
    library: Option<std::path::PathBuf>,
) -> Result<Vec<PageImage>, MediaError> {
    if cancelled {
        clear_render_temp(temp_dir);
        return Err(MediaError::new(CANCELLED, "PDF 渲染已取消"));
    }
    if timed_out {
        clear_render_temp(temp_dir);
        return Err(MediaError::new(TIMEOUT, "PDF 渲染超时"));
    }
    let selected = match pages_for_render(bytes, pages) {
        Ok(selected) => selected,
        Err(error) => {
            clear_render_temp(temp_dir);
            return Err(error);
        }
    };
    if selected.len() > MAX_PDF_RENDER_PAGES {
        clear_render_temp(temp_dir);
        return Err(MediaError::new(
            MEDIA_BUDGET_EXCEEDED,
            format!("一次最多渲染 {MAX_PDF_RENDER_PAGES} 页"),
        ));
    }
    let Some(library) = library.filter(|path| path.is_file()) else {
        clear_render_temp(temp_dir);
        return Err(pdfium_missing());
    };
    let rendered = render_with_pdfium(&library, bytes, &selected);
    clear_render_temp(temp_dir);
    rendered
}

fn pages_for_render(bytes: &[u8], pages: Option<&[u32]>) -> Result<Vec<u32>, MediaError> {
    if bytes.is_empty() || !bytes.starts_with(b"%PDF-") {
        return Err(MediaError::new(PDF_INVALID, "不是有效 PDF"));
    }
    let document = Document::load_mem(bytes)
        .map_err(|error| MediaError::new(PDF_INVALID, error.to_string()))?;
    if document.is_encrypted() {
        return Err(MediaError::new(PDF_ENCRYPTED, "PDF 已加密"));
    }
    let page_count = document.get_pages().len() as u32;
    if page_count == 0 {
        return Err(MediaError::new(PDF_INVALID, "PDF 没有页面"));
    }
    select_pages(page_count, pages)
}

fn render_with_pdfium(
    library: &std::path::Path,
    bytes: &[u8],
    pages: &[u32],
) -> Result<Vec<PageImage>, MediaError> {
    use pdfium_render::prelude::*;
    use std::sync::OnceLock;

    static ENGINE: OnceLock<Pdfium> = OnceLock::new();
    let pdfium = if let Some(existing) = ENGINE.get() {
        existing
    } else {
        let bindings = Pdfium::bind_to_library(library).map_err(|error| {
            MediaError::new(COMPONENT_MISSING, format!("无法加载 Pdfium: {error}"))
        })?;
        let _ = ENGINE.set(Pdfium::new(bindings));
        ENGINE.get().expect("pdfium engine")
    };
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|error| MediaError::new(PDF_INVALID, format!("PDF 无法渲染: {error}")))?;
    let mut images = Vec::with_capacity(pages.len());
    for page_number in pages {
        let page = document
            .pages()
            .get((*page_number as i32) - 1)
            .map_err(|error| {
                MediaError::new(PDF_INVALID, format!("第 {page_number} 页无法打开: {error}"))
            })?;
        let config = PdfRenderConfig::new().set_target_width(960);
        let bitmap = page.render_with_config(&config).map_err(|error| {
            MediaError::new(PDF_INVALID, format!("第 {page_number} 页渲染失败: {error}"))
        })?;
        let dynamic = bitmap.as_image().map_err(|error| {
            MediaError::new(PDF_INVALID, format!("第 {page_number} 页无法编码: {error}"))
        })?;
        let mut png = Vec::new();
        dynamic
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|error| {
                MediaError::new(PDF_INVALID, format!("第 {page_number} 页无法编码: {error}"))
            })?;
        images.push(PageImage {
            page: *page_number,
            png,
        });
    }
    Ok(images)
}

pub(crate) fn native_images_for_pages(
    pages: Vec<PageImage>,
) -> Vec<crate::native::model::types::NativeImage> {
    use base64::Engine;
    pages
        .into_iter()
        .map(|page| crate::native::model::types::NativeImage {
            name: format!("page-{}.png", page.page),
            mime_type: "image/png".to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(page.png),
            attachment_id: String::new(),
            page: Some(page.page),
            time_range: None,
        })
        .collect()
}

fn clear_render_temp(temp_dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(temp_dir);
}

fn select_pages(page_count: u32, pages: Option<&[u32]>) -> Result<Vec<u32>, MediaError> {
    let Some(pages) = pages else {
        if page_count > MAX_NATIVE_PDF_PAGES {
            return Err(MediaError::new(
                RANGE_REQUIRED,
                format!("PDF 共 {page_count} 页，超过 {MAX_NATIVE_PDF_PAGES} 页时必须指定范围"),
            ));
        }
        return Ok((1..=page_count).collect());
    };
    if pages.is_empty() {
        return Err(MediaError::new(RANGE_REQUIRED, "页码范围不能为空"));
    }
    let mut selected = Vec::new();
    for page in pages {
        if *page == 0 || *page > page_count {
            return Err(MediaError::new(
                PDF_INVALID,
                format!("页码 {page} 超出 1..={page_count}"),
            ));
        }
        if !selected.contains(page) {
            selected.push(*page);
        }
    }
    selected.sort_unstable();
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::Content;
    use lopdf::{dictionary, Document, Object, Stream};

    fn sample_pdf(texts: &[&str]) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let mut kids = Vec::new();
        for text in texts {
            let content = Content {
                operations: vec![
                    lopdf::content::Operation::new("BT", vec![]),
                    lopdf::content::Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    lopdf::content::Operation::new("Td", vec![10.into(), 10.into()]),
                    lopdf::content::Operation::new(
                        "Tj",
                        vec![Object::string_literal((*text).to_string())],
                    ),
                    lopdf::content::Operation::new("ET", vec![]),
                ],
            };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
            });
            kids.push(page_id.into());
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => texts.len() as i64,
                "Resources" => resources_id,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn tc_pdf_001_text_keeps_selected_pages_in_order() {
        let bytes = sample_pdf(&["PAGE_1", "PAGE_2", "PAGE_3"]);
        let report = read_pdf_text(&bytes, Some(&[3, 1, 3])).unwrap();
        assert_eq!(report.page_count, 3);
        assert_eq!(report.pages, vec![1, 3]);
        assert!(report.text.contains("PAGE_1"));
        assert!(report.text.contains("PAGE_3"));
        assert!(!report.text.contains("PAGE_2"));
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn tc_pdf_003_rejects_empty_encrypted_and_damaged() {
        assert_eq!(read_pdf_text(b"", None).unwrap_err().code, PDF_INVALID);
        assert_eq!(read_pdf_text(b"hello", None).unwrap_err().code, PDF_INVALID);
        let empty = sample_pdf(&[""]);
        assert_eq!(read_pdf_text(&empty, None).unwrap_err().code, PDF_INVALID);
    }

    #[test]
    fn tc_pdf_004_out_of_range_does_not_take_a_prefix() {
        let bytes = sample_pdf(&["only"]);
        assert_eq!(
            read_pdf_text(&bytes, Some(&[0])).unwrap_err().code,
            PDF_INVALID
        );
        assert_eq!(
            read_pdf_text(&bytes, Some(&[2])).unwrap_err().code,
            PDF_INVALID
        );
        assert_eq!(
            read_pdf_text(&bytes, Some(&[])).unwrap_err().code,
            RANGE_REQUIRED
        );
    }

    #[test]
    fn tc_pdf_006_page_render_reports_limits_and_clears_temp_files() {
        let bytes = sample_pdf(&["PAGE_1", "PAGE_2"]);
        let dir = tempfile::tempdir().unwrap();
        let render_dir = dir.path().join("render");
        std::fs::create_dir_all(&render_dir).unwrap();
        std::fs::write(render_dir.join("page-1.png"), b"leftover").unwrap();
        let missing =
            render_page_images_with(&bytes, Some(&[2, 1]), &render_dir, false, false, None)
                .unwrap_err();
        assert_eq!(missing.code, COMPONENT_MISSING);
        assert!(missing.message.contains("Pdfium"));
        assert!(!render_dir.exists());
        assert!(!dir.path().join("page-1.png").exists());

        let cancel_dir = dir.path().join("cancel");
        std::fs::create_dir_all(&cancel_dir).unwrap();
        std::fs::write(cancel_dir.join("page-9.png"), b"x").unwrap();
        assert_eq!(
            render_page_images_with(&bytes, Some(&[1]), &cancel_dir, true, false, None)
                .unwrap_err()
                .code,
            CANCELLED
        );
        assert!(!cancel_dir.exists());

        let timeout_dir = dir.path().join("timeout");
        std::fs::create_dir_all(&timeout_dir).unwrap();
        assert_eq!(
            render_page_images_with(&bytes, Some(&[1]), &timeout_dir, false, true, None)
                .unwrap_err()
                .code,
            TIMEOUT
        );
        assert!(!timeout_dir.exists());

        let mixed = sample_pdf(&["PAGE_1", ""]);
        let mixed_dir = dir.path().join("mixed");
        std::fs::create_dir_all(&mixed_dir).unwrap();
        std::fs::write(mixed_dir.join("page-1.png"), b"partial").unwrap();
        assert_eq!(
            render_page_images_with(&mixed, None, &mixed_dir, false, false, None)
                .unwrap_err()
                .code,
            COMPONENT_MISSING
        );
        assert!(!mixed_dir.exists());

        let range_dir = dir.path().join("range");
        assert_eq!(
            render_page_images_with(&bytes, Some(&[9]), &range_dir, false, false, None)
                .unwrap_err()
                .code,
            PDF_INVALID
        );
        let many = sample_pdf(&["p"; 9]);
        assert_eq!(
            render_page_images_with(&many, None, &dir.path().join("many"), false, false, None)
                .unwrap_err()
                .code,
            MEDIA_BUDGET_EXCEEDED
        );
        let scanned = sample_pdf(&[""]);
        assert_eq!(
            render_page_images_with(
                &scanned,
                Some(&[1]),
                &dir.path().join("scan"),
                false,
                false,
                None
            )
            .unwrap_err()
            .code,
            COMPONENT_MISSING
        );
    }

    #[test]
    fn tc_pdf_007_pdfium_renders_selected_pages_as_png() {
        let library = std::env::var_os("NOXCODE_PDFIUM")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file())
            .or_else(|| {
                let bundled = std::path::PathBuf::from("/tmp/nox-pdfium/lib/libpdfium.dylib");
                bundled.is_file().then_some(bundled)
            });
        let Some(library) = library else {
            return;
        };
        let bytes = sample_pdf(&["PAGE_1", "PAGE_2"]);
        let dir = tempfile::tempdir().unwrap();
        let render_dir = dir.path().join("render");
        std::fs::create_dir_all(&render_dir).unwrap();
        std::fs::write(render_dir.join("stale.png"), b"stale").unwrap();
        let images = render_page_images_with(
            &bytes,
            Some(&[2, 1]),
            &render_dir,
            false,
            false,
            Some(library),
        )
        .unwrap();
        assert!(!render_dir.exists());
        assert_eq!(
            images.iter().map(|page| page.page).collect::<Vec<_>>(),
            vec![1, 2]
        );
        for page in &images {
            assert!(page.png.starts_with(b"\x89PNG\r\n\x1a\n"));
            let decoded = image::load_from_memory(&page.png).unwrap();
            let colored = decoded
                .to_rgba8()
                .pixels()
                .any(|pixel| pixel.0[0] < 250 || pixel.0[1] < 250 || pixel.0[2] < 250);
            assert!(colored, "第 {} 页是空白图", page.page);
        }
        let native = native_images_for_pages(images);
        assert_eq!(native[0].page, Some(1));
        assert_eq!(native[0].mime_type, "image/png");
        assert!(native[0].data_url().starts_with("data:image/png;base64,"));
    }
}
