//! 可选运行组件的交付清单。缺失时只关闭对应能力，不把它们变成启动必装项。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;

use crate::native::media_error::{MediaError, COMPONENT_INCOMPATIBLE, COMPONENT_MISSING};

pub const PDFIUM_VERSION: &str = "chromium/8066";
pub const PDFIUM_RENDER_VERSION: &str = "0.9.4";

static RESOURCE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
struct Manifest {
    components: Vec<ComponentEntry>,
}

#[derive(Debug, Deserialize)]
struct ComponentEntry {
    id: String,
    role: String,
    delivery: String,
    version: String,
    #[serde(default)]
    render_crate: String,
    required_to_start: bool,
    platforms: Vec<String>,
    install: String,
    upgrade: String,
    missing: String,
    #[serde(default)]
    libraries: Vec<LibraryEntry>,
}

#[derive(Debug, Deserialize)]
struct LibraryEntry {
    target: String,
    file_name: String,
}

const MANIFEST: &str = include_str!("../../media-components.json");

fn manifest() -> Result<Manifest, MediaError> {
    serde_json::from_str(MANIFEST)
        .map_err(|error| MediaError::new(COMPONENT_MISSING, error.to_string()))
}

fn pdfium_entry(manifest: &Manifest) -> Result<&ComponentEntry, MediaError> {
    manifest
        .components
        .iter()
        .find(|item| item.id == "pdfium")
        .ok_or_else(|| MediaError::new(COMPONENT_MISSING, "清单里没有 Pdfium"))
}

pub(crate) fn remember_resource_dir(dir: &Path) {
    *RESOURCE_DIR.lock().expect("resource dir lock") = Some(dir.to_path_buf());
}

pub(crate) fn bundled_pdfium(resource_dir: &Path) -> Result<PathBuf, MediaError> {
    let manifest = manifest()?;
    let entry = pdfium_entry(&manifest)?;
    if entry.version != PDFIUM_VERSION || entry.render_crate != PDFIUM_RENDER_VERSION {
        return Err(MediaError::new(
            COMPONENT_INCOMPATIBLE,
            "Pdfium 清单版本不匹配",
        ));
    }
    let target = current_target();
    let library = entry
        .libraries
        .iter()
        .find(|item| item.target == target)
        .ok_or_else(|| MediaError::new(COMPONENT_INCOMPATIBLE, "当前平台没有 Pdfium 条目"))?;
    let path = resource_dir.join("pdfium").join(&library.file_name);
    if !path.is_file() {
        return Err(MediaError::new(
            COMPONENT_MISSING,
            format!("缺少包内组件 {}", library.file_name),
        ));
    }
    Ok(path)
}

pub(crate) fn pdfium_library_path() -> Result<PathBuf, MediaError> {
    if let Some(path) = std::env::var_os("NOXCODE_PDFIUM") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(MediaError::new(
            COMPONENT_MISSING,
            "NOXCODE_PDFIUM 不是可读的库文件",
        ));
    }
    let dir = RESOURCE_DIR.lock().expect("resource dir lock").clone();
    match dir {
        Some(dir) => bundled_pdfium(&dir),
        None => Err(MediaError::new(COMPONENT_MISSING, "无法读取应用资源目录")),
    }
}

pub(crate) fn pdfium_startup_notice(resource_dir: Option<&Path>) -> Option<String> {
    optional_component_notice(resource_dir, node_on_path())
}

pub(crate) fn optional_component_notice(
    resource_dir: Option<&Path>,
    node_present: bool,
) -> Option<String> {
    if let Some(dir) = resource_dir {
        remember_resource_dir(dir);
    }
    let mut parts = Vec::new();
    let pdfium_error = match resource_dir {
        Some(dir) => bundled_pdfium(dir).err(),
        None => Some(MediaError::new(COMPONENT_MISSING, "无法读取应用资源目录")),
    };
    if let Some(error) = pdfium_error {
        parts.push(component_repair_message(&error));
    }
    if !node_present {
        parts.push(
            "component_missing: 未检测到 node。浏览器自动化需要本机 Node.js，请先安装 Node，再在 MCP 设置里加入 Playwright 预设。安装包不携带 JS 运行时，应用本身仍可启动。文档预览使用已编译进应用的库。"
                .to_string(),
        );
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

pub(crate) fn component_repair_message(error: &MediaError) -> String {
    format!(
        "{}: {}。页面图使用随包 Pdfium {}（pdfium-render {}），不是系统必装组件。请把当前平台的库放到应用资源目录 pdfium/：Windows x64 为 pdfium.dll，Linux x64 为 libpdfium.so，macOS 为 libpdfium.dylib。升级时用清单中的同一版本替换该文件。缺少库时应用仍可启动，PDF 文本提取仍然可用。",
        error.code, error.message, PDFIUM_VERSION, PDFIUM_RENDER_VERSION
    )
}

fn node_on_path() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn current_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc_comp_002_missing_library_does_not_search_outside_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("pdftotext");
        std::fs::write(&outside, b"system").unwrap();
        let error = bundled_pdfium(dir.path()).unwrap_err();
        assert_eq!(error.code, COMPONENT_MISSING);
        assert!(outside.is_file());
        assert!(!error.message.contains("pdftotext"));
        let repair = component_repair_message(&error);
        assert!(repair.contains("pdfium/"));
        assert!(repair.contains("pdfium.dll"));
        assert!(repair.contains("libpdfium.so"));
        assert!(repair.contains("libpdfium.dylib"));
        assert!(repair.contains("仍可启动"));
        assert!(!repair.contains("PATH"));
        assert!(optional_component_notice(Some(dir.path()), true)
            .unwrap()
            .contains("component_missing"));
        let bundled = dir.path().join("pdfium");
        std::fs::create_dir_all(&bundled).unwrap();
        let file_name = match current_target() {
            "x86_64-pc-windows-msvc" => "pdfium.dll",
            "x86_64-unknown-linux-gnu" => "libpdfium.so",
            _ => "libpdfium.dylib",
        };
        if current_target() != "unsupported" {
            std::fs::write(bundled.join(file_name), b"library").unwrap();
            assert!(optional_component_notice(Some(dir.path()), true).is_none());
        }
    }

    #[test]
    fn tc_comp_003_manifest_covers_delivery_for_every_desktop_os() {
        let manifest = manifest().unwrap();
        let ids = ["pdfium", "pdfjs", "docx-preview", "playwright", "node"];
        for id in ids {
            let entry = manifest
                .components
                .iter()
                .find(|item| item.id == id)
                .unwrap_or_else(|| panic!("missing {id}"));
            assert!(!entry.version.is_empty(), "{id} version");
            assert!(!entry.install.is_empty(), "{id} install");
            assert!(!entry.upgrade.is_empty(), "{id} upgrade");
            assert!(!entry.missing.is_empty(), "{id} missing");
            assert!(!entry.required_to_start, "{id} must not block startup");
            for platform in ["windows", "macos", "linux"] {
                assert!(
                    entry.platforms.iter().any(|item| item == platform),
                    "{id} missing {platform}"
                );
            }
        }
        let pdfium = pdfium_entry(&manifest).unwrap();
        assert_eq!(pdfium.delivery, "bundled");
        assert_eq!(pdfium.role, "pdf-page-images");
        let playwright = manifest
            .components
            .iter()
            .find(|item| item.id == "playwright")
            .unwrap();
        assert_eq!(playwright.delivery, "on-demand");
        assert_eq!(playwright.role, "browser");
        let documents = manifest
            .components
            .iter()
            .find(|item| item.id == "docx-preview")
            .unwrap();
        assert_eq!(documents.delivery, "library");
        let node = optional_component_notice(Some(std::env::temp_dir().as_ref()), false).unwrap();
        assert!(node.contains("Node"));
        assert!(node.contains("Playwright"));
        assert!(node.contains("应用本身仍可启动"));
    }
}
