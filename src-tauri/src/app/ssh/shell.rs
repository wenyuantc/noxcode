use std::path::PathBuf;

#[allow(dead_code)]
pub(crate) fn shell_escape_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn shell_escape_double_quoted(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    )
}

#[allow(dead_code)]
pub(crate) fn remote_shell_path_expression(path: &str) -> String {
    let normalized = path.trim();
    if normalized.is_empty() {
        return "\"$HOME\"".to_string();
    }
    if matches!(normalized, "~" | "$HOME" | "${HOME}") {
        return "\"$HOME\"".to_string();
    }
    if let Some(rest) = normalized.strip_prefix("~/") {
        return format!(
            "\"$HOME/{}\"",
            shell_escape_double_quoted(rest).trim_matches('"')
        );
    }
    if let Some(rest) = normalized.strip_prefix("$HOME/") {
        return format!(
            "\"$HOME/{}\"",
            shell_escape_double_quoted(rest).trim_matches('"')
        );
    }
    if let Some(rest) = normalized.strip_prefix("${HOME}/") {
        return format!(
            "\"$HOME/{}\"",
            shell_escape_double_quoted(rest).trim_matches('"')
        );
    }
    shell_escape_double_quoted(normalized)
}

#[allow(dead_code)]
pub(crate) fn remote_path_join(base: &str, leaf: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.is_empty() {
        leaf.to_string()
    } else {
        format!("{trimmed}/{leaf}")
    }
}

#[allow(dead_code)]
pub(crate) fn redact_secret_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        "[REDACTED]".to_string()
    }
}

pub(crate) fn remote_shell_bootstrap() -> String {
    let statements = [
        "PATH=\"/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH\"",
        "for dir in \"$HOME/.local/bin\" \"$HOME/bin\"; do [ -d \"$dir\" ] && PATH=\"$dir:$PATH\"; done",
        "export PATH",
        "hash -r 2>/dev/null || true",
    ];
    format!("{}; ", statements.join("; "))
}

pub(crate) fn build_remote_shell_command(script: &str) -> String {
    format!(
        "sh -lc {}",
        shell_escape_single_quoted(&format!("{}{}", remote_shell_bootstrap(), script))
    )
}

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub(crate) fn expand_tilde(path: &str) -> PathBuf {
    let trimmed = path.trim();
    if trimmed == "~" {
        return user_home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return user_home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(trimmed));
    }
    PathBuf::from(trimmed)
}

pub(crate) fn current_username() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_quoted_escape_handles_apostrophe() {
        assert_eq!(shell_escape_single_quoted("abc"), "'abc'");
        assert_eq!(shell_escape_single_quoted("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn double_quoted_escape_handles_specials() {
        assert_eq!(shell_escape_double_quoted(r#"a"$`b"#), r#""a\"\$\`b""#);
        assert_eq!(shell_escape_double_quoted(r"c\d"), r#""c\\d""#);
    }

    #[test]
    fn remote_shell_path_expression_expands_home_prefix() {
        assert_eq!(remote_shell_path_expression("~"), "\"$HOME\"");
        assert_eq!(remote_shell_path_expression("$HOME"), "\"$HOME\"");
        assert_eq!(
            remote_shell_path_expression("~/codex sdk"),
            "\"$HOME/codex sdk\""
        );
        assert_eq!(
            remote_shell_path_expression("${HOME}/runtime"),
            "\"$HOME/runtime\""
        );
        assert_eq!(remote_shell_path_expression("/abs/path"), "\"/abs/path\"");
    }

    #[test]
    fn remote_path_join_trims_trailing_slash() {
        assert_eq!(remote_path_join("/tmp/", "a"), "/tmp/a");
        assert_eq!(remote_path_join("", "a"), "a");
        assert_eq!(remote_path_join("/tmp", "a"), "/tmp/a");
    }

    #[test]
    fn redact_secret_text_replaces_non_empty() {
        assert_eq!(redact_secret_text(""), "");
        assert_eq!(redact_secret_text("   "), "");
        assert_eq!(redact_secret_text("secret"), "[REDACTED]");
    }

    #[test]
    fn build_remote_shell_command_uses_sh_lc() {
        let command = build_remote_shell_command("uname -a");
        assert!(command.starts_with("sh -lc "));
        assert!(command.contains("/usr/bin"));
        assert!(command.contains("$HOME/.local/bin"));
        assert!(!command.contains("nvm"));
        assert!(!command.contains("pnpm"));
    }
}
