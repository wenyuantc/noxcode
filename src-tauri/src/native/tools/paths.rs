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
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("路径不能为空".to_string());
    }
    let root = normalize_logical_path(root);
    let candidate = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        root.join(trimmed)
    };
    let resolved = normalize_logical_path(&candidate);
    if !is_under_root(&root, &resolved) {
        return Err(format!("路径超出工作区: {trimmed}"));
    }
    Ok(resolved)
}

pub fn resolve_under_workspace_posix(root: &str, input: &str) -> Result<String, String> {
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
    let resolved = normalize_posix(&candidate);
    let root_normalized = normalize_posix(&root);
    if resolved != root_normalized && !resolved.starts_with(&format!("{root_normalized}/")) {
        return Err(format!("路径超出工作区: {trimmed}"));
    }
    Ok(resolved)
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
        assert_eq!(resolved, PathBuf::from("/tmp/ws/src/main.rs"));
        let nested = resolve_under_workspace(&root, "src/../README.md").unwrap();
        assert_eq!(nested, PathBuf::from("/tmp/ws/README.md"));
    }

    #[test]
    fn posix_escape_is_rejected() {
        let error = resolve_under_workspace_posix("/home/proj", "../etc/passwd").unwrap_err();
        assert!(error.contains("超出工作区"));
        let ok = resolve_under_workspace_posix("/home/proj", "lib/a.rs").unwrap();
        assert_eq!(ok, "/home/proj/lib/a.rs");
    }
}
