pub fn glob_match(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    let candidate = candidate.replace('\\', "/");
    match_glob(pattern.as_bytes(), candidate.as_bytes())
}

fn match_glob(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern.len() >= 2 && pattern[0] == b'*' && pattern[1] == b'*' {
        let rest = if pattern.len() >= 3 && pattern[2] == b'/' {
            &pattern[3..]
        } else {
            &pattern[2..]
        };
        if match_glob(rest, text) {
            return true;
        }
        for split in 0..text.len() {
            if match_glob(rest, &text[split + 1..]) {
                return true;
            }
        }
        return rest.is_empty();
    }
    if pattern[0] == b'*' {
        if match_glob(&pattern[1..], text) {
            return true;
        }
        return !text.is_empty() && text[0] != b'/' && match_glob(pattern, &text[1..]);
    }
    if text.is_empty() {
        return false;
    }
    let first_ok = pattern[0] == text[0] || (pattern[0] == b'?' && text[0] != b'/');
    first_ok && match_glob(&pattern[1..], &text[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_nested_rust_files() {
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "src/native/tools.rs"));
        assert!(!glob_match("**/*.rs", "src/main.toml"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/native/main.rs"));
    }
}
