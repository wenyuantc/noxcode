//! 为本机 MCP stdio 解析启动命令并补全 PATH。
//!
//! 桌面进程从 Dock / 启动器拉起时 PATH 通常只有系统目录，找不到 Homebrew、
//! nvm、fnm、volta 里的 `npx`。这里把常见用户 bin 并入搜索路径，再把裸命令
//! 解析成绝对路径；子进程仍注入补全后的 PATH，以便 `npx` 的 shebang 能找到 `node`。

use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::app::ssh::shell::{expand_tilde, user_home_dir};

pub(crate) fn command_not_found(command: &str) -> String {
    format!(
        "找不到命令 \"{command}\"。桌面进程 PATH 通常不含 Homebrew / nvm，请安装 Node.js 或把启动命令改成绝对路径。"
    )
}

pub(crate) fn augmented_path() -> OsString {
    join_augmented_path(
        &env::var_os("PATH").unwrap_or_default(),
        user_home_dir().as_deref(),
    )
}

pub(crate) fn resolve_program(command: &str) -> Result<PathBuf, String> {
    resolve_program_in(command, &augmented_path())
}

pub(crate) fn join_augmented_path(current: &OsStr, home: Option<&Path>) -> OsString {
    let mut dirs: Vec<PathBuf> = env::split_paths(current)
        .filter(|path| !path.as_os_str().is_empty())
        .collect();
    let mut seen: HashSet<PathBuf> = dirs.iter().cloned().collect();
    for extra in extra_path_dirs(home) {
        if extra.is_dir() && seen.insert(extra.clone()) {
            dirs.push(extra);
        }
    }
    env::join_paths(&dirs).unwrap_or_else(|_| current.to_os_string())
}

pub(crate) fn resolve_program_in(command: &str, search_path: &OsStr) -> Result<PathBuf, String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("MCP 启动命令不能为空".to_string());
    }
    if is_explicit_path(trimmed) {
        let expanded = expand_tilde(trimmed);
        if is_runnable(&expanded) {
            return Ok(expanded);
        }
        return Err(command_not_found(trimmed));
    }
    for dir in env::split_paths(search_path) {
        if let Some(found) = find_in_dir(&dir, trimmed) {
            return Ok(found);
        }
    }
    Err(command_not_found(trimmed))
}

fn is_explicit_path(command: &str) -> bool {
    Path::new(command).is_absolute()
        || command.starts_with('~')
        || command.contains('/')
        || command.contains('\\')
}

fn extra_path_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    #[cfg(windows)]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\nodejs"));
        dirs.push(PathBuf::from(r"C:\Program Files (x86)\nodejs"));
    }
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
        dirs.push(home.join(".volta/bin"));
        dirs.push(home.join(".asdf/shims"));
        if let Some(nvm_dir) = env::var_os("NVM_DIR") {
            dirs.extend(nvm_bin_dirs(Path::new(&nvm_dir)));
        }
        dirs.extend(nvm_bin_dirs(&home.join(".nvm")));
        dirs.extend(fnm_bin_dirs(home));
    }
    dirs
}

fn nvm_bin_dirs(nvm_dir: &Path) -> Vec<PathBuf> {
    if let Some(version) = resolve_nvm_alias(nvm_dir, "default", 0) {
        for candidate in [
            nvm_dir
                .join("versions")
                .join("node")
                .join(format!("v{version}"))
                .join("bin"),
            nvm_dir
                .join("versions")
                .join("node")
                .join(&version)
                .join("bin"),
        ] {
            if candidate.is_dir() {
                return vec![candidate];
            }
        }
    }
    latest_nvm_node_bin(nvm_dir).into_iter().collect()
}

fn resolve_nvm_alias(nvm_dir: &Path, name: &str, depth: u8) -> Option<String> {
    if depth > 6 || name.is_empty() {
        return None;
    }
    let text = std::fs::read_to_string(nvm_dir.join("alias").join(name)).ok()?;
    let value = text.lines().next()?.trim();
    if value.is_empty() || value == "system" {
        return None;
    }
    let stripped = value.strip_prefix('v').unwrap_or(value);
    if stripped.starts_with(|ch: char| ch.is_ascii_digit()) {
        return Some(stripped.to_string());
    }
    resolve_nvm_alias(nvm_dir, value, depth + 1)
}

fn latest_nvm_node_bin(nvm_dir: &Path) -> Option<PathBuf> {
    let root = nvm_dir.join("versions").join("node");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("bin").is_dir())
        .collect();
    entries.sort();
    entries.pop().map(|path| path.join("bin"))
}

fn fnm_bin_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(multishell) = env::var("FNM_MULTISHELL_PATH") {
        let path = PathBuf::from(multishell);
        if path.is_dir() {
            dirs.push(path);
        }
    }
    for candidate in [
        home.join(".fnm")
            .join("aliases")
            .join("default")
            .join("bin"),
        home.join(".local")
            .join("share")
            .join("fnm")
            .join("aliases")
            .join("default")
            .join("bin"),
    ] {
        if candidate.is_dir() {
            dirs.push(candidate);
        }
    }
    dirs
}

fn find_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if is_runnable(&direct) {
        return Some(direct);
    }
    for ext in pathext_suffixes() {
        let candidate = dir.join(format!("{name}{ext}"));
        if is_runnable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn pathext_suffixes() -> Vec<String> {
    #[cfg(windows)]
    {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(str::trim)
            .filter(|ext| !ext.is_empty())
            .map(|ext| {
                if ext.starts_with('.') {
                    ext.to_string()
                } else {
                    format!(".{ext}")
                }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn is_runnable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = env::temp_dir().join(format!("noxcode-command-path-{stamp}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn write_fake_bin(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parent");
        }
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").expect("write bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).expect("chmod");
        }
    }

    fn with_home<T>(home: &Path, func: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let old_home = env::var_os("HOME");
        let old_profile = env::var_os("USERPROFILE");
        env::set_var("HOME", home);
        env::set_var("USERPROFILE", home);
        let result = func();
        match old_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        match old_profile {
            Some(value) => env::set_var("USERPROFILE", value),
            None => env::remove_var("USERPROFILE"),
        }
        result
    }

    #[test]
    fn absolute_path_is_returned_as_is() {
        let dir = temp_dir();
        let bin = dir.join("custom-npx");
        write_fake_bin(&bin);
        let resolved =
            resolve_program_in(bin.to_str().expect("utf8"), OsStr::new("")).expect("abs");
        assert_eq!(resolved, bin);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tilde_path_expands_to_home_bin() {
        let home = temp_dir();
        let bin = home.join("bin").join("npx");
        write_fake_bin(&bin);
        let resolved = with_home(&home, || {
            resolve_program_in("~/bin/npx", OsStr::new("")).expect("tilde")
        });
        assert_eq!(resolved, bin);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn resolves_bare_name_from_search_path() {
        let dir = temp_dir();
        let bin = dir.join("npx");
        write_fake_bin(&bin);
        let resolved = resolve_program_in("npx", dir.as_os_str()).expect("npx");
        assert_eq!(resolved, bin);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolves_from_augmented_home_bin_when_path_empty() {
        let home = temp_dir();
        let bin = home.join(".local").join("bin").join("npx");
        write_fake_bin(&bin);
        let search = join_augmented_path(OsStr::new("/does/not/exist"), Some(&home));
        let resolved = resolve_program_in("npx", &search).expect("augmented npx");
        assert_eq!(resolved, bin);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn augmented_path_includes_home_local_bin() {
        let home = temp_dir();
        let local_bin = home.join(".local").join("bin");
        std::fs::create_dir_all(&local_bin).expect("mkdir");
        let joined = join_augmented_path(OsStr::new("/only/here"), Some(&home));
        let dirs: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert!(dirs.contains(&PathBuf::from("/only/here")));
        assert!(dirs.contains(&local_bin));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn nvm_default_alias_bin_is_added() {
        let home = temp_dir();
        let bin = home
            .join(".nvm")
            .join("versions")
            .join("node")
            .join("v20.11.0")
            .join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir nvm");
        std::fs::create_dir_all(home.join(".nvm").join("alias")).expect("mkdir alias");
        std::fs::write(home.join(".nvm").join("alias").join("default"), "20.11.0\n")
            .expect("alias");
        let joined = join_augmented_path(OsStr::new("/x"), Some(&home));
        assert!(env::split_paths(&joined).any(|dir| dir == bin));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn missing_command_mentions_name() {
        let err = resolve_program_in("definitely-missing-mcp-bin", OsStr::new("/empty"))
            .expect_err("missing");
        assert!(err.contains("找不到命令"));
        assert!(err.contains("definitely-missing-mcp-bin"));
    }

    #[cfg(windows)]
    #[test]
    fn resolves_npx_cmd_via_pathext() {
        let dir = temp_dir();
        let cmd = dir.join("npx.cmd");
        std::fs::write(&cmd, b"@echo off\r\n").expect("write cmd");
        let resolved = resolve_program_in("npx", dir.as_os_str()).expect("npx.cmd");
        assert_eq!(resolved, cmd);
        let _ = std::fs::remove_dir_all(dir);
    }
}
