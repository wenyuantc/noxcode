use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

pub fn normalize_logical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

pub fn resolve_under_workspace(root: &Path, input: &str) -> Result<PathBuf, String> {
    let resolved = resolve_local_path(root, input)?;
    let physical_root = resolve_local_path(root, ".")?;
    if !is_under_root(&physical_root, &resolved) {
        return Err(format!("路径超出工作区: {}", input.trim()));
    }
    Ok(resolved)
}

/// Resolve the physical target independently of the authorization boundary.
pub fn resolve_local_path(root: &Path, input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("路径不能为空".to_string());
    }
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("读取当前目录失败: {error}"))?
            .join(root)
    };
    let candidate = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        root.join(trimmed)
    };
    // 不先折叠 `..`：符号链接后的父目录必须遵循文件系统语义。
    canonicalize_allow_missing(&candidate)
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf, String> {
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(format!("无法解析符号链接: {}", path.display()));
            }
            let parent = path
                .parent()
                .filter(|parent| *parent != path)
                .ok_or_else(|| format!("无法解析路径: {}", path.display()))?;
            let physical_parent = canonicalize_allow_missing(parent)?;
            let component = path
                .components()
                .next_back()
                .ok_or_else(|| format!("无法解析路径: {}", path.display()))?;
            Ok(normalize_logical_path(
                &physical_parent.join(component.as_os_str()),
            ))
        }
        Err(error) => Err(format!("无法解析路径 {}: {error}", path.display())),
    }
}

pub fn resolve_under_workspace_posix(root: &str, input: &str) -> Result<String, String> {
    let resolved = resolve_posix_path(root, input)?;
    let root_normalized = resolve_posix_path(root, ".")?;
    if !path_is_within(&root_normalized, &resolved) {
        return Err(format!("路径超出工作区: {}", input.trim()));
    }
    Ok(resolved)
}

pub fn resolve_posix_path(root: &str, input: &str) -> Result<String, String> {
    let root = trim_slash(root);
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("路径不能为空".to_string());
    }
    let candidate = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("{root}/{trimmed}")
    };
    Ok(normalize_posix(&candidate))
}

pub fn path_is_within(root: &str, candidate: &str) -> bool {
    if root.as_bytes().get(1) == Some(&b':') || root.starts_with("\\\\") {
        let root = root.replace('\\', "/");
        let candidate = candidate.replace('\\', "/");
        return candidate == root
            || candidate.starts_with(&format!("{}/", root.trim_end_matches('/')));
    }
    candidate == root || candidate.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

fn is_under_root(root: &Path, candidate: &Path) -> bool {
    if candidate == root {
        return true;
    }
    candidate.starts_with(root)
}

fn trim_slash(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() > 1 {
        trimmed.trim_end_matches('/').to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_posix(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let absolute = path.starts_with('/');
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                let _ = parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_parent_escape() {
        let root = PathBuf::from("/tmp/ws");
        let error = resolve_under_workspace(&root, "../secret.txt").unwrap_err();
        assert!(error.contains("超出工作区"));
        let error = resolve_under_workspace(&root, "/etc/passwd").unwrap_err();
        assert!(error.contains("超出工作区"));
    }

    #[test]
    fn allows_relative_and_nested_paths() {
        let root = PathBuf::from("/tmp/ws");
        let resolved = resolve_under_workspace(&root, "src/main.rs").unwrap();
        assert_eq!(
            resolved,
            canonicalize_allow_missing(&root)
                .unwrap()
                .join("src/main.rs")
        );
        let nested = resolve_under_workspace(&root, "src/../README.md").unwrap();
        assert_eq!(
            nested,
            canonicalize_allow_missing(&root).unwrap().join("README.md")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_for_existing_and_new_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        symlink(outside.path().join("missing"), root.path().join("dangling")).unwrap();
        for path in [
            "link/secret",
            "link/new/nested.txt",
            "link/../outside.txt",
            "dangling",
        ] {
            assert!(
                resolve_under_workspace(root.path(), path).is_err(),
                "{path}"
            );
        }
        fs::create_dir(root.path().join("inside")).unwrap();
        symlink(root.path().join("inside"), root.path().join("safe")).unwrap();
        assert_eq!(
            resolve_under_workspace(root.path(), "safe/new.txt").unwrap(),
            fs::canonicalize(root.path())
                .unwrap()
                .join("inside/new.txt")
        );
    }

    #[test]
    fn posix_escape_is_rejected() {
        let error = resolve_under_workspace_posix("/home/proj", "../etc/passwd").unwrap_err();
        assert!(error.contains("超出工作区"));
        let ok = resolve_under_workspace_posix("/home/proj", "lib/a.rs").unwrap();
        assert_eq!(ok, "/home/proj/lib/a.rs");
        assert_eq!(
            resolve_under_workspace_posix("/", "etc/hosts").unwrap(),
            "/etc/hosts"
        );
    }
}
