//! 表格和 Word 文档只向模型和预览提供提取出的文本。

use std::io::{Cursor, Read};

use calamine::Reader;
use serde::Serialize;

use crate::native::media_limits::TEXT_BYTE_BUDGET;

const PREVIEW_MAX_SHEETS: usize = 8;
const PREVIEW_MAX_ROWS: usize = 200;
const PREVIEW_MAX_COLS: usize = 32;

#[derive(Debug, Clone, Serialize)]
pub struct OfficePreview {
    pub kind: String,
    pub sheets: Vec<OfficeSheet>,
    pub paragraphs: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OfficeSheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

const OLE_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub(crate) fn office_mime(extension: Option<&str>) -> Option<&'static str> {
    match extension? {
        "xls" => Some("application/vnd.ms-excel"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "doc" => Some("application/msword"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        _ => None,
    }
}

pub(crate) fn office_bytes_match(extension: Option<&str>, bytes: &[u8]) -> bool {
    match extension {
        Some("xlsx" | "docx") => is_zip(bytes),
        Some("xls" | "doc") => is_ole(bytes),
        _ => false,
    }
}

pub(crate) fn extract_office_text(name: &str, bytes: &[u8]) -> Result<String, String> {
    let preview = read_office(name, bytes, false)?;
    let text = match preview.kind.as_str() {
        "sheet" => sheets_to_text(&preview.sheets),
        _ => preview.paragraphs.join("\n"),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("文档里没有可读内容，不能当作已读取".to_string());
    }
    if preview.truncated {
        return Ok(limit_text(trimmed));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn preview_office(name: &str, bytes: &[u8]) -> Result<OfficePreview, String> {
    read_office(name, bytes, true)
}

fn read_office(name: &str, bytes: &[u8], preview: bool) -> Result<OfficePreview, String> {
    let extension = extension_of(name);
    if !office_bytes_match(extension.as_deref(), bytes) {
        return Err("扩展名与内容不符".to_string());
    }
    match extension.as_deref() {
        Some("xls" | "xlsx") => {
            let (sheets, truncated) = read_sheets(
                bytes,
                if preview {
                    PREVIEW_MAX_SHEETS
                } else {
                    usize::MAX
                },
                if preview {
                    PREVIEW_MAX_ROWS
                } else {
                    usize::MAX
                },
                if preview {
                    PREVIEW_MAX_COLS
                } else {
                    usize::MAX
                },
            )?;
            if sheets.iter().all(|sheet| sheet.rows.is_empty()) {
                return Err("表格里没有可读内容，不能当作已读取".to_string());
            }
            Ok(OfficePreview {
                kind: "sheet".to_string(),
                sheets,
                paragraphs: Vec::new(),
                truncated,
            })
        }
        Some("docx") => document_preview(extract_docx(bytes)?),
        Some("doc") => document_preview(extract_doc(bytes)?),
        _ => Err("不支持的文档类型".to_string()),
    }
}

fn document_preview(text: String) -> Result<OfficePreview, String> {
    let limited = limit_text(text.trim());
    let truncated = limited.contains("（文本已截断，未完整读取）");
    let body = limited.trim_end_matches("\n（文本已截断，未完整读取）");
    let paragraphs = body
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        return Err("文档里没有可读内容，不能当作已读取".to_string());
    }
    Ok(OfficePreview {
        kind: "document".to_string(),
        sheets: Vec::new(),
        paragraphs,
        truncated,
    })
}

fn sheets_to_text(sheets: &[OfficeSheet]) -> String {
    let mut out = String::new();
    for sheet in sheets {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("# ");
        out.push_str(&sheet.name);
        out.push('\n');
        for row in &sheet.rows {
            out.push_str(&row.join("\t"));
            out.push('\n');
        }
    }
    out
}

fn extension_of(name: &str) -> Option<String> {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = file_name.rsplit('.').next()?;
    if ext == file_name {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
}

fn is_ole(bytes: &[u8]) -> bool {
    bytes.starts_with(OLE_MAGIC)
}

/// 部分 Mac 写出的复合文档把 FAT 扇区标成链尾。读之前改成规范里的 FAT 标记。
fn relax_ole_fat(bytes: &[u8]) -> Vec<u8> {
    const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
    const FAT_SECTOR: u32 = 0xFFFF_FFFD;
    const MAX_REGULAR: u32 = 0xFFFF_FFFA;
    if !is_ole(bytes) || bytes.len() < 0x4C + 109 * 4 {
        return bytes.to_vec();
    }
    let sector_shift = u16::from_le_bytes(bytes[0x1E..0x20].try_into().unwrap_or([0, 0]));
    if !(9..=12).contains(&sector_shift) {
        return bytes.to_vec();
    }
    let sector_size = 1usize << sector_shift;
    let mut fat_sectors = Vec::new();
    for index in 0..109 {
        let offset = 0x4C + index * 4;
        let id = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0xFF; 4]));
        if id <= MAX_REGULAR {
            fat_sectors.push(id);
        }
    }
    let mut out = bytes.to_vec();
    let per_sector = sector_size / 4;
    let mini_fat_count = u32::from_le_bytes(out[0x40..0x44].try_into().unwrap_or([0; 4])) as usize;
    let dir_start = u32::from_le_bytes(out[0x30..0x34].try_into().unwrap_or([0xFF; 4]));
    if mini_fat_count > 0 && dir_start <= MAX_REGULAR {
        let needed = (mini_fat_count * per_sector) as u64 * 64;
        let header = if sector_size == 512 { 512 } else { sector_size };
        let len_off = header + dir_start as usize * sector_size + 120;
        if let Some(slot) = out.get(len_off..len_off + 8) {
            let current = u64::from_le_bytes(slot.try_into().unwrap_or([0; 8]));
            if current < needed {
                out[len_off..len_off + 8].copy_from_slice(&needed.to_le_bytes());
            }
        }
    }
    for &sid in &fat_sectors {
        let which = sid as usize / per_sector;
        let within = sid as usize % per_sector;
        let Some(&fat_sid) = fat_sectors.get(which) else {
            continue;
        };
        let header = if sector_size == 512 { 512 } else { sector_size };
        let offset = header + fat_sid as usize * sector_size + within * 4;
        let Some(slot) = out.get(offset..offset + 4) else {
            continue;
        };
        let value = u32::from_le_bytes(slot.try_into().unwrap_or([0; 4]));
        if value == ENDOFCHAIN {
            out[offset..offset + 4].copy_from_slice(&FAT_SECTOR.to_le_bytes());
        }
    }
    out
}

fn limit_text(text: &str) -> String {
    if text.len() <= TEXT_BYTE_BUDGET {
        return text.to_string();
    }
    let mut end = TEXT_BYTE_BUDGET;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut clipped = text[..end].to_string();
    clipped.push_str("\n（文本已截断，未完整读取）");
    clipped
}

fn read_sheets(
    bytes: &[u8],
    max_sheets: usize,
    max_rows: usize,
    max_cols: usize,
) -> Result<(Vec<OfficeSheet>, bool), String> {
    let prepared;
    let bytes = if is_ole(bytes) {
        prepared = relax_ole_fat(bytes);
        prepared.as_slice()
    } else {
        bytes
    };
    let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|error| format!("表格无法读取，可能已加密或已损坏: {error}"))?;
    let names = workbook.sheet_names();
    if names.is_empty() {
        return Err("表格里没有工作表".to_string());
    }
    let mut sheets = Vec::new();
    let mut truncated = names.len() > max_sheets;
    let mut bytes_used = 0usize;
    for name in names.into_iter().take(max_sheets) {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| format!("无法读取工作表 {name}: {error}"))?;
        let mut rows = Vec::new();
        for row in range.rows() {
            if rows.len() >= max_rows {
                truncated = true;
                break;
            }
            let mut cells: Vec<String> = row.iter().take(max_cols).map(cell_text).collect();
            if row.len() > max_cols {
                truncated = true;
            }
            while cells.last().is_some_and(|cell| cell.trim().is_empty()) {
                cells.pop();
            }
            if cells.iter().all(|cell| cell.trim().is_empty()) {
                continue;
            }
            bytes_used += cells.iter().map(String::len).sum::<usize>();
            rows.push(cells);
            if bytes_used > TEXT_BYTE_BUDGET {
                truncated = true;
                break;
            }
        }
        sheets.push(OfficeSheet { name, rows });
        if bytes_used > TEXT_BYTE_BUDGET {
            break;
        }
    }
    Ok((sheets, truncated))
}

fn cell_text(cell: &calamine::Data) -> String {
    match cell {
        calamine::Data::Empty | calamine::Data::Error(_) => String::new(),
        other => other.to_string(),
    }
}

fn extract_docx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Word 文档无法读取: {error}"))?;
    let mut xml = String::new();
    let file = archive
        .by_name("word/document.xml")
        .map_err(|_| "Word 文档缺少正文".to_string())?;
    file.take(32 * 1024 * 1024)
        .read_to_string(&mut xml)
        .map_err(|error| format!("Word 文档无法读取: {error}"))?;
    Ok(docx_xml_to_text(&xml))
}

fn docx_xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while !rest.is_empty() {
        if rest.starts_with("</w:p>") || rest.starts_with("</w:tr>") {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            rest = &rest[if rest.starts_with("</w:p>") { 6 } else { 7 }..];
            continue;
        }
        if rest.starts_with("<w:tab") {
            out.push('\t');
            rest = skip_tag(rest);
            continue;
        }
        if rest.starts_with("<w:br") {
            out.push('\n');
            rest = skip_tag(rest);
            continue;
        }
        if is_word_text_tag(rest) {
            let Some(tag_end) = rest.find('>') else { break };
            if tag_end > 0 && rest.as_bytes()[tag_end - 1] == b'/' {
                rest = &rest[tag_end + 1..];
                continue;
            }
            let start = tag_end + 1;
            let Some(text_end) = rest[start..].find("</w:t>") else {
                break;
            };
            out.push_str(&decode_xml(&rest[start..start + text_end]));
            rest = &rest[start + text_end + 6..];
            continue;
        }
        rest = &rest[rest.chars().next().map(|ch| ch.len_utf8()).unwrap_or(1)..];
    }
    out
}

fn is_word_text_tag(xml: &str) -> bool {
    xml.starts_with("<w:t>") || xml.starts_with("<w:t ") || xml.starts_with("<w:t/")
}

fn skip_tag(xml: &str) -> &str {
    match xml.find('>') {
        Some(end) => &xml[end + 1..],
        None => "",
    }
}

fn decode_xml(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let token = &rest[1..end];
        let ch = if let Some(hex) = token
            .strip_prefix("#x")
            .or_else(|| token.strip_prefix("#X"))
        {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = token.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            Some(match token {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => '\u{FFFD}',
            })
        };
        out.push(ch.unwrap_or('\u{FFFD}'));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn extract_doc(bytes: &[u8]) -> Result<String, String> {
    let ole = OleFile::open(bytes)?;
    let word = ole.stream("WordDocument")?;
    if word.len() < 0x20 || read_u16(&word, 0)? != 0xA5EC {
        return Err("不是有效的 Word 文档".to_string());
    }
    let flags = read_u16(&word, 0x0A)?;
    if flags & 0x0100 != 0 {
        return Err("文档已加密".to_string());
    }
    let table_name = if flags & 0x0200 != 0 {
        "1Table"
    } else {
        "0Table"
    };
    let (fc_clx, lcb_clx, ccp_text) = fib_clx(&word)?;
    let table = ole.stream(table_name)?;
    piece_text(&word, &table, fc_clx, lcb_clx, ccp_text)
}

struct OleFile<'a> {
    bytes: &'a [u8],
    sector_size: usize,
    fat: Vec<u32>,
}

impl<'a> OleFile<'a> {
    fn open(bytes: &'a [u8]) -> Result<Self, String> {
        if !is_ole(bytes) || bytes.len() < 0x200 {
            return Err("不是有效的 Word 文档".to_string());
        }
        let sector_shift = u16::from_le_bytes(bytes[0x1E..0x20].try_into().unwrap_or([0, 0]));
        if !(9..=12).contains(&sector_shift) {
            return Err("不是有效的 Word 文档".to_string());
        }
        let sector_size = 1usize << sector_shift;
        let mut fat_sectors = Vec::new();
        for index in 0..109 {
            let offset = 0x4C + index * 4;
            let id = read_u32(bytes, offset).unwrap_or(0xFFFF_FFFF);
            if id <= 0xFFFF_FFFA {
                fat_sectors.push(id);
            }
        }
        let mut fat = Vec::new();
        for sid in fat_sectors {
            let chunk = sector_bytes(bytes, sid, sector_size)?;
            for entry in chunk.chunks_exact(4) {
                fat.push(u32::from_le_bytes(entry.try_into().unwrap_or([0; 4])));
            }
        }
        Ok(Self {
            bytes,
            sector_size,
            fat,
        })
    }

    fn stream(&self, name: &str) -> Result<Vec<u8>, String> {
        let dir_start = read_u32(self.bytes, 0x30).unwrap_or(0xFFFF_FFFF);
        let directory = self.chain(dir_start, None)?;
        for entry in directory.chunks_exact(128) {
            let name_len = u16::from_le_bytes(entry[64..66].try_into().unwrap_or([0, 0])) as usize;
            if !(2..=64).contains(&name_len) {
                continue;
            }
            let label = String::from_utf16_lossy(
                &entry[..name_len - 2]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>(),
            );
            if label != name {
                continue;
            }
            let start = u32::from_le_bytes(entry[116..120].try_into().unwrap_or([0; 4]));
            let size = u32::from_le_bytes(entry[120..124].try_into().unwrap_or([0; 4])) as usize;
            return self.chain(start, Some(size));
        }
        Err(format!("Word 文档缺少 {name}"))
    }

    fn chain(&self, start: u32, size: Option<usize>) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut sid = start;
        let mut seen = 0usize;
        let limit = size.unwrap_or(1024 * 1024);
        while sid <= 0xFFFF_FFFA
            && out.len() < limit.saturating_add(self.sector_size)
            && seen < 10_000
        {
            out.extend_from_slice(sector_bytes(self.bytes, sid, self.sector_size)?);
            sid = *self.fat.get(sid as usize).ok_or("文档损坏")?;
            seen += 1;
        }
        if let Some(size) = size {
            if out.len() < size {
                return Err("文档损坏".to_string());
            }
            out.truncate(size);
        }
        Ok(out)
    }
}

fn sector_bytes(bytes: &[u8], sid: u32, sector_size: usize) -> Result<&[u8], String> {
    let header = if sector_size == 512 { 512 } else { sector_size };
    let offset = header + sid as usize * sector_size;
    bytes
        .get(offset..offset + sector_size)
        .ok_or_else(|| "文档损坏".to_string())
}

fn fib_clx(word: &[u8]) -> Result<(usize, usize, usize), String> {
    let mut offset = 32usize;
    let shorts = read_u16(word, offset)? as usize;
    offset = offset.checked_add(2 + shorts * 2).ok_or("文档损坏")?;
    let longs = read_u16(word, offset)? as usize;
    offset = offset.checked_add(2).ok_or("文档损坏")?;
    let ccp_text = read_i32(word, offset.checked_add(12).ok_or("文档损坏")?)?;
    if ccp_text <= 0 {
        return Err("文档里没有可读内容，不能当作已读取".to_string());
    }
    offset = offset.checked_add(longs * 4).ok_or("文档损坏")?;
    let pairs = read_u16(word, offset)? as usize;
    offset = offset.checked_add(2).ok_or("文档损坏")?;
    if pairs <= 33 {
        return Err("这份 Word 文档的版本无法读取".to_string());
    }
    let pair = offset.checked_add(33 * 8).ok_or("文档损坏")?;
    let fc = read_i32(word, pair)?;
    let lcb = read_i32(word, pair + 4)?;
    if fc < 0 || lcb <= 0 {
        return Err("文档里没有可读内容，不能当作已读取".to_string());
    }
    Ok((fc as usize, lcb as usize, ccp_text as usize))
}

fn piece_text(
    word: &[u8],
    table: &[u8],
    fc: usize,
    lcb: usize,
    ccp_text: usize,
) -> Result<String, String> {
    let clx = table
        .get(fc..fc.checked_add(lcb).ok_or("文档损坏")?)
        .ok_or("文档损坏")?;
    let mut index = 0usize;
    while index < clx.len() {
        match clx[index] {
            1 => {
                let cb = read_u16(clx, index + 1)? as usize;
                index = index.checked_add(3 + cb).ok_or("文档损坏")?;
            }
            2 => {
                let plc_len = read_i32(clx, index + 1)? as usize;
                let start = index.checked_add(5).ok_or("文档损坏")?;
                let plc = clx
                    .get(start..start.checked_add(plc_len).ok_or("文档损坏")?)
                    .ok_or("文档损坏")?;
                return decode_pieces(word, plc, ccp_text);
            }
            _ => return Err("Word 正文结构损坏".to_string()),
        }
    }
    Err("文档里没有可读内容，不能当作已读取".to_string())
}

fn decode_pieces(word: &[u8], plc: &[u8], ccp_text: usize) -> Result<String, String> {
    if plc.len() < 16 {
        return Err("Word 正文结构损坏".to_string());
    }
    let count = (plc.len() - 4) / 12;
    let mut out = String::new();
    for piece in 0..count {
        let cp = read_i32(plc, piece * 4)? as usize;
        let next = read_i32(plc, (piece + 1) * 4)? as usize;
        if cp >= ccp_text || next <= cp {
            continue;
        }
        let chars = next.min(ccp_text) - cp;
        let pcd = (count + 1) * 4 + piece * 8;
        let fc_raw = read_u32(plc, pcd + 2)?;
        let unicode = fc_raw & 0x4000_0000 == 0;
        let mut pos = (fc_raw & 0x3FFF_FFFF) as usize;
        if !unicode {
            pos /= 2;
        }
        append_piece(word, pos, chars, unicode, &mut out)?;
        if out.len() > TEXT_BYTE_BUDGET {
            break;
        }
    }
    Ok(out)
}

fn append_piece(
    word: &[u8],
    pos: usize,
    chars: usize,
    unicode: bool,
    out: &mut String,
) -> Result<(), String> {
    if unicode {
        let end = pos.checked_add(chars * 2).ok_or("文档损坏")?;
        let bytes = word.get(pos..end).ok_or("文档损坏")?;
        for pair in bytes.chunks(2) {
            let unit = u16::from_le_bytes([pair[0], pair.get(1).copied().unwrap_or(0)]);
            push_word_char(
                out,
                char::decode_utf16([unit])
                    .next()
                    .unwrap_or(Ok('\u{FFFD}'))
                    .unwrap_or('\u{FFFD}'),
            );
        }
    } else {
        let end = pos.checked_add(chars).ok_or("文档损坏")?;
        let bytes = word.get(pos..end).ok_or("文档损坏")?;
        for &byte in bytes {
            push_word_char(out, byte as char);
        }
    }
    Ok(())
}

fn push_word_char(out: &mut String, ch: char) {
    match ch {
        '\r' | '\u{000B}' => out.push('\n'),
        '\u{0007}' => out.push('\t'),
        '\t' | '\n' => out.push(ch),
        ch if (ch as u32) < 32 => {}
        ch => out.push(ch),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes.get(offset..offset + 2).ok_or("文档损坏")?;
    Ok(u16::from_le_bytes(
        slice.try_into().map_err(|_| "文档损坏")?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or("文档损坏")?;
    Ok(u32::from_le_bytes(
        slice.try_into().map_err(|_| "文档损坏")?,
    ))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    Ok(read_u32(bytes, offset)? as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(files: &[(&str, &str)]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        for (name, body) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        cursor.into_inner()
    }

    #[test]
    fn xlsx_cells_keep_sheet_name_and_order() {
        let bytes = zip_with(&[
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
                  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
                  <Default Extension="xml" ContentType="application/xml"/>
                  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
                  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
                </Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
                </Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
                  <sheets><sheet name="指标" sheetId="1" r:id="rId1"/></sheets>
                </workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
                </Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                  <sheetData>
                    <row r="1">
                      <c r="A1" t="inlineStr"><is><t>名称</t></is></c>
                      <c r="B1" t="inlineStr"><is><t>数量</t></is></c>
                    </row>
                    <row r="2">
                      <c r="A2" t="inlineStr"><is><t>指标</t></is></c>
                      <c r="B2"><v>12</v></c>
                    </row>
                  </sheetData>
                </worksheet>"#,
            ),
        ]);
        let text = extract_office_text("indicator_import_template.xlsx", &bytes).unwrap();
        assert!(text.contains("# 指标"));
        assert!(text.contains("名称\t数量"));
        assert!(text.contains("指标\t12"));
        let preview = preview_office("indicator_import_template.xlsx", &bytes).unwrap();
        assert_eq!(preview.kind, "sheet");
        assert_eq!(preview.sheets[0].name, "指标");
        assert_eq!(preview.sheets[0].rows[0], vec!["名称", "数量"]);
        assert_eq!(preview.sheets[0].rows[1][1], "12");
        assert!(extract_office_text("notes.txt", &bytes).is_err());
    }

    #[test]
    fn docx_paragraphs_keep_text_and_escapes() {
        let bytes = zip_with(&[(
            "word/document.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p><w:r><w:t>合同&amp;正文</w:t></w:r></w:p>
                <w:p><w:r><w:t>第二段</w:t></w:r></w:p>
              </w:body>
            </w:document>"#,
        )]);
        let text = extract_office_text("说明.docx", &bytes).unwrap();
        assert!(text.contains("合同&正文"));
        assert!(text.contains("第二段"));
        let styled = zip_with(&[(
            "word/document.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:tbl><w:tr><w:tc><w:p><w:r>
                  <w:rPr><w:rFonts w:hAnsi="CGFFUA+MicrosoftYaHei-Bold" w:cs="CGFFUA+MicrosoftYaHei-Bold"/>
                  <w:sz w:val="24"/></w:rPr>
                  <w:t/>
                  <w:t>保密</w:t>
                </w:r></w:p></w:tc></w:tr></w:tbl>
                <w:p><w:r><w:t>微众信科</w:t></w:r></w:p>
              </w:body>
            </w:document>"#,
        )]);
        let styled_text = extract_office_text("接口.docx", &styled).unwrap();
        assert!(styled_text.contains("保密"));
        assert!(styled_text.contains("微众信科"));
        assert!(!styled_text.contains("w:hAnsi"));
        assert!(!styled_text.contains("w:sz"));
        assert!(!styled_text.contains("<w:"));
        let styled_preview = preview_office("接口.docx", &styled).unwrap();
        assert!(styled_preview.paragraphs.iter().any(|line| line == "保密"));
        assert!(styled_preview
            .paragraphs
            .iter()
            .all(|line| !line.contains("w:") && !line.contains('<')));
        let preview = preview_office("说明.docx", &bytes).unwrap();
        assert_eq!(preview.kind, "document");
        assert!(preview
            .paragraphs
            .iter()
            .any(|line| line.contains("合同&正文")));
    }

    #[test]
    fn doc_piece_table_reads_unicode_text() {
        let mut word = vec![0u8; 4096];
        word[0] = 0xEC;
        word[1] = 0xA5;
        word[0x0A] = 0x00;
        word[0x0B] = 0x02;
        put_u16(&mut word, 32, 0);
        put_u16(&mut word, 34, 16);
        put_i32(&mut word, 48, 2);
        put_u16(&mut word, 100, 93);
        put_i32(&mut word, 366, 0);
        put_i32(&mut word, 370, 21);
        let chars = [0x4F60u16, 0x597D];
        for (index, unit) in chars.into_iter().enumerate() {
            put_u16(&mut word, 1024 + index * 2, unit);
        }
        let mut table = vec![2u8];
        table.extend_from_slice(&16i32.to_le_bytes());
        table.extend_from_slice(&0i32.to_le_bytes());
        table.extend_from_slice(&2i32.to_le_bytes());
        table.extend_from_slice(&0u16.to_le_bytes());
        table.extend_from_slice(&1024u32.to_le_bytes());
        table.extend_from_slice(&0u16.to_le_bytes());
        table.resize(4096, 0);
        let bytes = ole_with(&word, &table);
        let text = extract_office_text("说明.doc", &bytes).unwrap();
        assert!(text.contains("你好"));
    }

    #[test]
    fn textutil_doc_round_trip_when_available() {
        let output = std::process::Command::new("textutil").arg("-help").output();
        if output.is_err() {
            return;
        }
        let path = std::env::temp_dir().join("noxcode-office-sample.doc");
        let mut child = match std::process::Command::new("textutil")
            .args(["-convert", "doc", "-stdin", "-output"])
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return,
        };
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all("合同正文".as_bytes())
            .unwrap();
        if !child.wait().unwrap().success() {
            return;
        }
        let bytes = std::fs::read(&path).unwrap();
        let text = extract_office_text("合同.doc", &bytes).unwrap_or_else(|error| error);
        assert!(
            text.contains("合同正文"),
            "extracted {text:?} from {} bytes",
            bytes.len()
        );
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn ole_with(word: &[u8], table: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut compound = cfb::CompoundFile::create(cursor).unwrap();
        compound
            .create_stream("WordDocument")
            .unwrap()
            .write_all(word)
            .unwrap();
        compound
            .create_stream("1Table")
            .unwrap()
            .write_all(table)
            .unwrap();
        compound.into_inner().into_inner()
    }
}
