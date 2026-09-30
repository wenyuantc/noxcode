//! `Memory` 工具：只读写绑定给当前 Agent 的记忆目录。
//!
//! 记忆整理 Agent 与带持久记忆的子 Agent 通过它维护记忆，不需要 Write / Edit，
//! 也就不会因此获得工作区写权限。文件名只能是目录里的直接子文件，
//! 符号链接一律拒绝。

use std::path::PathBuf;

use serde_json::Value;

use super::dispatch::ToolCtx;
use crate::native::memory;

/// 当前 Agent 可访问的记忆目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBinding {
    pub dir: PathBuf,
    pub writable: bool,
}

pub fn call(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let binding = ctx
        .memory
        .as_ref()
        .ok_or_else(|| "当前 Agent 没有可用的记忆目录".to_string())?;
    let args: Value =
        serde_json::from_str(arguments).map_err(|error| format!("参数不是有效 JSON: {error}"))?;
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let dir = &binding.dir;
    let action = text("action");
    if matches!(action.as_str(), "write" | "delete") && !binding.writable {
        return Err("记忆目录是只读的".to_string());
    }
    match action.as_str() {
        "list" => {
            let entries = memory::list_entries(dir);
            if entries.is_empty() {
                return Ok("（还没有记忆）".to_string());
            }
            Ok(entries
                .iter()
                .map(|entry| {
                    format!(
                        "- {} | {} | {} | 更新于 {} — {}",
                        entry.file_name,
                        entry.name,
                        entry.kind,
                        entry.updated_at,
                        entry.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "search" => {
            let query = text("query");
            if query.is_empty() {
                return Err("search 需要 query".to_string());
            }
            let hits = memory::recall(dir, &query, 10);
            if hits.is_empty() {
                return Ok("没有匹配的记忆".to_string());
            }
            Ok(hits
                .iter()
                .map(|hit| format!("- {} | {} — {}", hit.file_name, hit.name, hit.description))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "read" => {
            let file = checked_file(&text("file"))?;
            let entry = memory::read_entry(dir, &file).ok_or_else(|| format!("没有记忆 {file}"))?;
            Ok(format!(
                "文件：{}\n名称：{}\n类型：{}\n描述：{}\n创建：{}\n更新：{}\n\n{}",
                entry.file_name,
                entry.name,
                entry.kind,
                entry.description,
                entry.created_at,
                entry.updated_at,
                entry.body
            ))
        }
        "write" => {
            let name = text("name");
            let body = text("body");
            if name.is_empty() || body.is_empty() {
                return Err("write 需要 name 和 body".to_string());
            }
            let file = match text("file") {
                file if file.is_empty() => None,
                file => {
                    let file = checked_file(&file)?;
                    memory::read_entry(dir, &file)
                        .ok_or_else(|| format!("要更新的记忆 {file} 不存在"))?;
                    Some(file)
                }
            };
            let entry = memory::save_entry_in(
                dir,
                file.as_deref(),
                &name,
                &text("type"),
                &text("description"),
                &body,
            )?;
            Ok(format!("已保存记忆 {}（{}）", entry.file_name, entry.name))
        }
        "delete" => {
            let file = checked_file(&text("file"))?;
            if memory::delete_entry(dir, &file)? {
                Ok(format!("已删除记忆 {file}"))
            } else {
                Err(format!("没有记忆 {file}"))
            }
        }
        other => Err(format!(
            "未知的 action：{other}（可用 list / search / read / write / delete）"
        )),
    }
}

fn checked_file(file: &str) -> Result<String, String> {
    if memory::is_entry_file_name(file) {
        Ok(file.to_string())
    } else {
        Err(format!("无效的记忆文件名：{file}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tools::LocalWorkspace;

    fn ctx(writable: bool) -> (ToolCtx, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "noxcode-memory-tool-{}",
            crate::native::artifacts::unique_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut ctx = ToolCtx::new(LocalWorkspace::new(dir.clone()));
        ctx.memory = Some(MemoryBinding {
            dir: dir.clone(),
            writable,
        });
        (ctx, dir)
    }

    fn run(ctx: &ToolCtx, args: serde_json::Value) -> Result<String, String> {
        call(ctx, &args.to_string())
    }

    #[test]
    fn actions_read_and_write_only_the_bound_directory() {
        let (ctx, dir) = ctx(true);
        let saved = run(
            &ctx,
            serde_json::json!({"action":"write","name":"构建命令","type":"project",
                "description":"怎么构建","body":"cargo build"}),
        )
        .unwrap();
        assert!(saved.contains(".md"));
        let file = memory::list_entries(&dir)[0].file_name.clone();
        assert!(run(&ctx, serde_json::json!({"action":"list"}))
            .unwrap()
            .contains(&file));
        assert!(
            run(&ctx, serde_json::json!({"action":"search","query":"构建"}))
                .unwrap()
                .contains("构建命令")
        );
        // 传 file 原地更新并改名。
        run(
            &ctx,
            serde_json::json!({"action":"write","file":file,"name":"构建与测试命令",
                "body":"cargo build && cargo test"}),
        )
        .unwrap();
        let read = run(&ctx, serde_json::json!({"action":"read","file":file})).unwrap();
        assert!(read.contains("构建与测试命令") && read.contains("cargo test"));
        assert_eq!(memory::list_entries(&dir).len(), 1);
        for escape in ["../outside.md", "sub/x.md", "MEMORY.md", ".state.json"] {
            assert!(run(&ctx, serde_json::json!({"action":"read","file":escape})).is_err());
            assert!(run(
                &ctx,
                serde_json::json!({"action":"write","file":escape,"name":"x","body":"y"})
            )
            .is_err());
        }
        assert!(!dir.parent().unwrap().join("outside.md").exists());
        assert!(run(&ctx, serde_json::json!({"action":"delete","file":file})).is_ok());
        assert!(memory::list_entries(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn read_only_binding_and_missing_binding_are_rejected() {
        let (ctx, dir) = ctx(false);
        assert!(run(&ctx, serde_json::json!({"action":"list"})).is_ok());
        assert!(run(
            &ctx,
            serde_json::json!({"action":"write","name":"x","body":"y"})
        )
        .unwrap_err()
        .contains("只读"));
        let mut unbound = ctx.clone();
        unbound.memory = None;
        assert!(run(&unbound, serde_json::json!({"action":"list"})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
