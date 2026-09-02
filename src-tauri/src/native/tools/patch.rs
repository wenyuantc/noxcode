use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchAction {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<Hunk>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<HunkLine>,
    pub eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileMutation {
    Write { path: String, content: String },
    Delete { path: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PatchCounts {
    pub add: usize,
    pub update: usize,
    pub delete: usize,
}

impl PatchCounts {
    pub fn total(self) -> usize {
        self.add + self.update + self.delete
    }

    pub fn summary(self) -> String {
        format!(
            "应用补丁：{} 个文件（新增 {} / 修改 {} / 删除 {}）",
            self.total(),
            self.add,
            self.update,
            self.delete
        )
    }
}

pub fn extract_patch_text(arguments: &str) -> Result<String, String> {
    let value: Value = if arguments.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(arguments)
            .map_err(|error| format!("工具参数不是合法 JSON: {error}"))?
    };
    value
        .get("patch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "patch 不能为空".to_string())
}

pub fn patch_counts(actions: &[PatchAction]) -> PatchCounts {
    let mut counts = PatchCounts::default();
    for action in actions {
        match action {
            PatchAction::Add { .. } => counts.add += 1,
            PatchAction::Delete { .. } => counts.delete += 1,
            PatchAction::Update { .. } => counts.update += 1,
        }
    }
    counts
}

pub fn parse_patch(input: &str) -> Result<Vec<PatchAction>, String> {
    let text = input.replace("\r\n", "\n");
    let lines: Vec<&str> = text.lines().collect();
    let begin = lines
        .iter()
        .position(|line| line.trim() == "*** Begin Patch")
        .ok_or_else(|| "补丁缺少 *** Begin Patch".to_string())?;
    let end = lines
        .iter()
        .rposition(|line| line.trim() == "*** End Patch")
        .ok_or_else(|| "补丁缺少 *** End Patch".to_string())?;
    if end <= begin {
        return Err("补丁 Begin/End 标记顺序无效".to_string());
    }
    let body = lines[begin + 1..end].to_vec();
    let mut actions = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let line = body[index].trim_end();
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if let Some(path) = strip_prefix(line, "*** Add File:") {
            let path = require_path(path, "Add File")?;
            index += 1;
            let (content, next) = collect_add_content(&body, index)?;
            actions.push(PatchAction::Add { path, content });
            index = next;
            continue;
        }
        if let Some(path) = strip_prefix(line, "*** Delete File:") {
            let path = require_path(path, "Delete File")?;
            actions.push(PatchAction::Delete { path });
            index += 1;
            continue;
        }
        if let Some(path) = strip_prefix(line, "*** Update File:") {
            let path = require_path(path, "Update File")?;
            index += 1;
            let mut move_to = None;
            if index < body.len() {
                if let Some(dest) = strip_prefix(body[index].trim_end(), "*** Move to:") {
                    move_to = Some(require_path(dest, "Move to")?);
                    index += 1;
                }
            }
            let (hunks, next) = collect_hunks(&body, index)?;
            if hunks.is_empty() && move_to.is_none() {
                return Err(format!("更新文件 {path} 缺少 hunk 或 Move to"));
            }
            actions.push(PatchAction::Update {
                path,
                move_to,
                hunks,
            });
            index = next;
            continue;
        }
        return Err(format!("无法识别的补丁行: {line}"));
    }
    if actions.is_empty() {
        return Err("补丁没有文件操作".to_string());
    }
    Ok(actions)
}

pub fn plan_mutations(
    actions: &[PatchAction],
    mut load: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<FileMutation>, String> {
    let mut mutations = Vec::new();
    for action in actions {
        match action {
            PatchAction::Add { path, content } => {
                if load(path)?.is_some() {
                    return Err(format!("无法新增：文件已存在 {path}"));
                }
                mutations.push(FileMutation::Write {
                    path: path.clone(),
                    content: normalize_file_content(content),
                });
            }
            PatchAction::Delete { path } => {
                if load(path)?.is_none() {
                    return Err(format!("无法删除：文件不存在 {path}"));
                }
                mutations.push(FileMutation::Delete { path: path.clone() });
            }
            PatchAction::Update {
                path,
                move_to,
                hunks,
            } => {
                let original = load(path)?.ok_or_else(|| format!("无法更新：文件不存在 {path}"))?;
                let updated = apply_hunks(&original, hunks, path)?;
                let dest = move_to.as_deref().unwrap_or(path);
                if dest != path && load(dest)?.is_some() {
                    return Err(format!("无法移动：目标已存在 {dest}"));
                }
                mutations.push(FileMutation::Write {
                    path: dest.to_string(),
                    content: updated,
                });
                if dest != path {
                    mutations.push(FileMutation::Delete { path: path.clone() });
                }
            }
        }
    }
    Ok(mutations)
}

fn collect_add_content(body: &[&str], mut index: usize) -> Result<(String, usize), String> {
    let mut lines = Vec::new();
    while index < body.len() {
        let raw = body[index];
        if raw.trim_start().starts_with("*** ") {
            break;
        }
        if let Some(rest) = raw.strip_prefix('+') {
            lines.push(rest.to_string());
        } else if raw.is_empty() {
            lines.push(String::new());
        } else {
            return Err(format!("新增文件内容必须以 + 开头: {raw}"));
        }
        index += 1;
    }
    Ok((lines.join("\n"), index))
}

fn collect_hunks(body: &[&str], mut index: usize) -> Result<(Vec<Hunk>, usize), String> {
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;
    while index < body.len() {
        let raw = body[index];
        let trimmed = raw.trim_end();
        if trimmed.starts_with("*** Add File:")
            || trimmed.starts_with("*** Delete File:")
            || trimmed.starts_with("*** Update File:")
        {
            break;
        }
        if trimmed == "*** End of File" {
            if let Some(hunk) = current.as_mut() {
                hunk.eof = true;
            } else {
                return Err("*** End of File 必须跟在 hunk 后面".to_string());
            }
            index += 1;
            continue;
        }
        if trimmed.starts_with("*** ") {
            return Err(format!("更新文件中出现未知标记: {trimmed}"));
        }
        if trimmed.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            current = Some(Hunk {
                header: trimmed.trim_start_matches("@@").trim().to_string(),
                lines: Vec::new(),
                eof: false,
            });
            index += 1;
            continue;
        }
        if current.is_none() {
            current = Some(Hunk {
                header: String::new(),
                lines: Vec::new(),
                eof: false,
            });
        }
        let hunk = current.as_mut().expect("hunk");
        if let Some(rest) = raw.strip_prefix('+') {
            hunk.lines.push(HunkLine::Add(rest.to_string()));
        } else if let Some(rest) = raw.strip_prefix('-') {
            hunk.lines.push(HunkLine::Remove(rest.to_string()));
        } else if let Some(rest) = raw.strip_prefix(' ') {
            hunk.lines.push(HunkLine::Context(rest.to_string()));
        } else if raw.is_empty() {
            hunk.lines.push(HunkLine::Context(String::new()));
        } else {
            hunk.lines.push(HunkLine::Context(raw.to_string()));
        }
        index += 1;
    }
    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }
    Ok((hunks, index))
}

fn apply_hunks(original: &str, hunks: &[Hunk], path: &str) -> Result<String, String> {
    let ended_with_nl = original.ends_with('\n');
    let mut lines: Vec<String> = original.split('\n').map(ToOwned::to_owned).collect();
    if ended_with_nl && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    for (index, hunk) in hunks.iter().enumerate() {
        lines = apply_one_hunk(&lines, hunk, path, index + 1)?;
    }
    let mut out = lines.join("\n");
    if ended_with_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn apply_one_hunk(
    lines: &[String],
    hunk: &Hunk,
    path: &str,
    hunk_no: usize,
) -> Result<Vec<String>, String> {
    let old = hunk_old_lines(hunk);
    let new = hunk_new_lines(hunk);
    if old.is_empty() {
        return insert_only(lines, hunk, &new, path, hunk_no);
    }
    let start = find_block(lines, &old, false)
        .or_else(|_| find_block(lines, &old, true))
        .map_err(|_| {
            format!(
                "补丁上下文不匹配：{path} 第 {hunk_no} 个 hunk\n{}",
                old.iter()
                    .take(6)
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })?;
    if hunk.eof && start + old.len() != lines.len() {
        return Err(format!("补丁要求匹配文件末尾：{path} 第 {hunk_no} 个 hunk"));
    }
    let mut out = Vec::with_capacity(lines.len() - old.len() + new.len());
    out.extend(lines[..start].iter().cloned());
    out.extend(new);
    out.extend(lines[start + old.len()..].iter().cloned());
    Ok(out)
}

fn insert_only(
    lines: &[String],
    hunk: &Hunk,
    new: &[String],
    path: &str,
    hunk_no: usize,
) -> Result<Vec<String>, String> {
    let at = if hunk.eof || hunk.header.is_empty() {
        lines.len()
    } else {
        lines
            .iter()
            .position(|line| line.contains(&hunk.header))
            .map(|index| index + 1)
            .ok_or_else(|| {
                format!(
                    "补丁插入点未找到：{path} 第 {hunk_no} 个 hunk（{}）",
                    hunk.header
                )
            })?
    };
    let mut out = Vec::with_capacity(lines.len() + new.len());
    out.extend(lines[..at].iter().cloned());
    out.extend(new.iter().cloned());
    out.extend(lines[at..].iter().cloned());
    Ok(out)
}

fn find_block(lines: &[String], old: &[String], fuzzy: bool) -> Result<usize, ()> {
    if old.len() > lines.len() {
        return Err(());
    }
    let mut found = None;
    for start in 0..=lines.len() - old.len() {
        if matches_at(lines, start, old, fuzzy) {
            if found.is_some() {
                return Err(());
            }
            found = Some(start);
        }
    }
    found.ok_or(())
}

fn matches_at(lines: &[String], start: usize, old: &[String], fuzzy: bool) -> bool {
    old.iter().enumerate().all(|(offset, expected)| {
        let actual = &lines[start + offset];
        if fuzzy {
            actual.trim_end() == expected.trim_end()
        } else {
            actual == expected
        }
    })
}

fn hunk_old_lines(hunk: &Hunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(text) | HunkLine::Remove(text) => Some(text.clone()),
            HunkLine::Add(_) => None,
        })
        .collect()
}

fn hunk_new_lines(hunk: &Hunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(text) | HunkLine::Add(text) => Some(text.clone()),
            HunkLine::Remove(_) => None,
        })
        .collect()
}

fn normalize_file_content(content: &str) -> String {
    if content.is_empty() || content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{content}\n")
    }
}

fn strip_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.trim().strip_prefix(prefix)
}

fn require_path(value: &str, kind: &str) -> Result<String, String> {
    let path = value.trim();
    if path.is_empty() {
        return Err(format!("{kind} 路径不能为空"));
    }
    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn load_from(
        map: &HashMap<String, String>,
    ) -> impl FnMut(&str) -> Result<Option<String>, String> + '_ {
        |path| Ok(map.get(path).cloned())
    }

    fn writes(mutations: &[FileMutation]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for item in mutations {
            match item {
                FileMutation::Write { path, content } => {
                    out.insert(path.clone(), content.clone());
                }
                FileMutation::Delete { path } => {
                    out.remove(path);
                }
            }
        }
        out
    }

    #[test]
    fn add_update_delete_and_move() {
        let patch = r#"*** Begin Patch
*** Add File: src/new.rs
+fn hi() {}
*** Update File: src/old.rs
@@ fn greet() {
-    println!("hi");
+    println!("hello");
*** Update File: src/rename.rs
*** Move to: src/renamed.rs
@@
-old
+new
*** Delete File: gone.txt
*** End Patch"#;
        let actions = parse_patch(patch).expect("parse");
        assert_eq!(patch_counts(&actions).add, 1);
        assert_eq!(patch_counts(&actions).update, 2);
        assert_eq!(patch_counts(&actions).delete, 1);
        let mut files = HashMap::new();
        files.insert(
            "src/old.rs".to_string(),
            "fn greet() {\n    println!(\"hi\");\n}\n".to_string(),
        );
        files.insert("src/rename.rs".to_string(), "old\n".to_string());
        files.insert("gone.txt".to_string(), "x\n".to_string());
        let mutations = plan_mutations(&actions, load_from(&files)).expect("plan");
        let out = writes(&mutations);
        assert_eq!(out.get("src/new.rs").expect("new"), "fn hi() {}\n");
        assert!(out
            .get("src/old.rs")
            .expect("old")
            .contains("println!(\"hello\")"));
        assert_eq!(out.get("src/renamed.rs").expect("renamed"), "new\n");
        assert!(mutations.iter().any(|item| matches!(
            item,
            FileMutation::Delete { path } if path == "gone.txt"
        )));
        assert!(mutations.iter().any(|item| matches!(
            item,
            FileMutation::Delete { path } if path == "src/rename.rs"
        )));
    }

    #[test]
    fn fuzzy_trailing_whitespace_then_mismatch_errors() {
        let patch = r#"*** Begin Patch
*** Update File: a.txt
@@
-hello
+hello world
*** End Patch"#;
        let actions = parse_patch(patch).expect("parse");
        let mut files = HashMap::new();
        files.insert("a.txt".to_string(), "hello   \n".to_string());
        let mutations = plan_mutations(&actions, load_from(&files)).expect("fuzzy");
        assert_eq!(writes(&mutations).get("a.txt").expect("a"), "hello world\n");

        files.insert("a.txt".to_string(), "nope\n".to_string());
        let err = plan_mutations(&actions, load_from(&files)).expect_err("mismatch");
        assert!(err.contains("上下文不匹配"));
        assert!(err.contains("a.txt"));
    }

    #[test]
    fn missing_markers_and_empty_patch() {
        assert!(parse_patch("not a patch")
            .unwrap_err()
            .contains("Begin Patch"));
        let err = parse_patch("*** Begin Patch\n*** End Patch").unwrap_err();
        assert!(err.contains("没有文件操作"));
    }

    #[test]
    fn extract_patch_argument() {
        assert!(extract_patch_text("{}").is_err());
        let text = extract_patch_text(r#"{"patch":"*** Begin Patch\n*** End Patch"}"#).unwrap();
        assert!(text.contains("Begin Patch"));
    }
}
