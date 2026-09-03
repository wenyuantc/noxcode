# 数据层

noxcode 用一份 SQLite 库保存渠道、工作区、会话与 Git checkpoint。前端不碰 SQL。

```
React (UI) → Tauri IPC commands → Rust service layer → SQLite
```

源码入口：

| 路径 | 职责 |
| --- | --- |
| [`src-tauri/src/db/migrations.rs`](../src-tauri/src/db/migrations.rs) | 迁移清单（version 1 baseline + version 2 去掉档案 + version 3 `agent_sessions.title` + version 4 `agent_sessions.pinned`） |
| [`src-tauri/src/db/models.rs`](../src-tauri/src/db/models.rs) | 行模型与 IPC DTO |
| [`src-tauri/src/app/shared.rs`](../src-tauri/src/app/shared.rs) | `sqlite_pool` / `database_path` / `now_sqlite` / `new_id` |
| [`src-tauri/src/app/database.rs`](../src-tauri/src/app/database.rs) | 健康检查、备份、恢复 |

`src/lib/database.ts` 是 hard-fail stub。`src-tauri/capabilities/default.json` 不授予任何 `sql:*` 权限。

## 文件位置

插件 preload URL 是 `sqlite:noxcode.db`，实际文件在 `$APPCONFIG/noxcode.db`：

| 平台 | 路径 |
| --- | --- |
| macOS | `~/Library/Application Support/com.wenyuan.noxcode/noxcode.db` |
| Linux | `~/.config/com.wenyuan.noxcode/noxcode.db` |
| Windows | `%APPDATA%\com.wenyuan.noxcode\noxcode.db` |

时间戳格式为 `%Y-%m-%d %H:%M:%S`（UTC）。主键是 UUID 字符串。

## 迁移

`tauri-plugin-sql` 在启动时按 `get_all_migrations()` 升级。版本号必须连续 `1..N`，由 `migration_versions_are_contiguous` 强制。当前最新版本是 **4**（`agent_sessions.pinned`），跑完后共 8 张业务表。

后续只能追加 `version: 5`……，禁止改已发布的 SQL，禁止插队。

应用的 `_sqlx_migrations` 表记录已应用版本。debug 启动会打印：

```
[db] 迁移检查完成: applied_count=1, current_version=1, latest_registered_version=1, ...
```

## 表关系

```
ai_channels ──SET NULL──► agent_sessions
ssh_configs ──RESTRICT──► workspaces     ──SET NULL──► agent_sessions
ssh_configs ──SET NULL──► agent_sessions
agent_sessions ──CASCADE──► agent_session_events
agent_sessions ──CASCADE──► git_checkpoints
workspaces     ──CASCADE──► git_checkpoints

native_api_call_logs        无外键（日志需在会话删除后仍可查）
native_session_transcripts  无外键（主键 session_record_id 对应 agent_sessions.id）
```

RESTRICT：仍被工作区引用的 SSH 配置不能删。  
渠道删除会把会话上的 `ai_channel_id` 置空；有该渠道的 live session 时命令层拒绝删除。  
CASCADE：删会话会清事件和 checkpoint 行；删工作区会清该工作区的 checkpoint 行。  
`git update-ref -d` 清仓库 ref 属于 Git 层，不在本模块。

## 八张表

### `ssh_configs`

SSH 连接配置。密码与密钥口令只存 keyring 引用（`password_ref` / `passphrase_ref`），不落明文。

| 列 | 说明 |
| --- | --- |
| `id` | 主键 |
| `name` / `host` / `port` / `username` | 连接参数，`port` 默认 22 |
| `auth_type` | `key` 或 `password` |
| `private_key_path` | 私钥路径（key 认证） |
| `password_ref` / `passphrase_ref` | keyring 引用 |
| `known_hosts_mode` | `accept-new`（默认）/ `strict` / `ask` / `off` |
| `last_checked_*` | 最近连通探测 |
| `password_probe_*` | 密码认证探测；`passed` / `available` 时 DTO 的 `password_execution_allowed` 为 true |
| `created_at` / `updated_at` | 更新触发器会刷新 `updated_at` |

对外 DTO 是 `SshConfig`：去掉 `*_ref`，改暴露 `password_configured` / `passphrase_configured` / `password_execution_allowed`。

### `ai_channels`

模型渠道。API Key 直接落 `api_key` 列（明文在库内），不是 keyring。

| 列 | 说明 |
| --- | --- |
| `protocol` | `openai` / `anthropic` / `codex`（后续渠道层校验） |
| `base_url` | 网关根地址 |
| `api_key` | 可空 |
| `extra_headers_json` | 额外请求头 |
| `models_json` | 模型列表 JSON，默认 `[]` |
| `enabled` | `1` / `0` |

### `workspaces`

本地或 SSH 工作目录。

| 列 | 说明 |
| --- | --- |
| `workspace_type` | `local` 或 `ssh` |
| `repo_path` | 本地路径 |
| `ssh_config_id` | SSH 工作区引用配置，删除受 RESTRICT |
| `remote_repo_path` | 远端仓库路径 |

### `agent_sessions`

一次 Agent 运行记录。没有 `task_id` / `cli_session_id` / `ai_provider` 等 CLI 引擎列。

| 列 | 说明 |
| --- | --- |
| `ai_channel_id` / `workspace_id` | 删除渠道或工作区时置空 |
| `working_dir` | 实际工作目录 |
| `execution_target` | `local` 或 `ssh` |
| `ssh_config_id` / `target_host_label` | SSH 会话元数据，配置删除时 `ssh_config_id` 置空 |
| `session_kind` | 默认 `execution` |
| `status` | 默认 `pending` |
| `started_at` / `ended_at` / `exit_code` | 生命周期 |
| `resume_session_id` | 续聊来源 |
| `title` | 会话标题。新建取首句最多 30 个 Unicode 字；续聊继承来源会话，不覆盖。version 3 从最早一条 `[USER_INPUT]` / `[用户输入]` 回填 |
| `pinned` | 侧栏置顶。`0` 未置顶，`1` 已置顶。version 4 新增，默认 `0`。取消置顶后仍按 `workspace_id` 归队 |
| `input_tokens` / `output_tokens` / `total_tokens` / `reasoning_tokens` / `cached_tokens` | 用量 |

### `agent_session_events`

会话事件流。`session_id` 级联删除。`event_type` + `message` 由会话层写入。

### `native_api_call_logs`

模型调用日志，便于排渠道问题。范围列用 `channel_id` / `workspace_id`，没有 `task_id`。不建外键，删会话后日志仍在。`profile_id` 列仍在，新写入为 NULL。

### `native_session_transcripts`

续聊上下文。主键 `session_record_id` 对应 `agent_sessions.id`。`messages_json` 存消息数组。`deleted_at` 软删。

### `git_checkpoints`

Git plumbing 快照的数据库索引。真实对象在仓库 `refs/noxcode/checkpoints/<session_id>/<seq>`。

| 列 | 说明 |
| --- | --- |
| `session_id` / `workspace_id` | 级联删除 |
| `seq` | 会话内序号；`(session_id, seq)` 唯一 |
| `ref_name` / `commit_oid` / `parent_oid` | ref 与对象 |
| `label` | 如「会话开始」 |
| `kind` | `session_start` / `after_tool_call` / `manual` / `auto_pre_restore` |

## 已注册命令

| 命令 | 作用 |
| --- | --- |
| `health_check` | 库是否加载、当前/最新迁移版本、系统 git 是否 ≥ 2.23 |
| `backup_database` | 导出 SQL 脚本（含 schema、数据、`_sqlx_migrations`） |
| `restore_database` | 校验脚本 → 写 `noxcode.pre-import-backup-*.sql` → 清库导入 → 补迁移 → 完整性检查 |
| `open_database_folder` | 用系统文件管理器打开 `$APPCONFIG` |
| `list_ssh_configs` / `get_ssh_config` / `create_ssh_config` / `update_ssh_config` / `delete_ssh_config` | SSH 配置 CRUD，见 [`ssh.md`](ssh.md) |
| `probe_ssh_password_auth` / `test_ssh_connection` | 写 `password_probe_*` / `last_check_*` |

备份范围只覆盖 SQLite 本体，包括工作区、SSH 配置、`ai_channels`（**API 密钥在库内，会随 SQL 导出**）、会话与事件、Git checkpoint 元数据、API 调用日志，以及 `_sqlx_migrations`。不包括：

- keyring 里的 SSH 密码 / 私钥口令（服务名 `noxcode-ssh`）
- 应用配置目录里的 `ssh-secret-index.json`
- 应用配置目录里的 `mcp-servers.json`、`native-settings.json`（含钩子）、`network-settings.json`、`quick-prompts.json`
- 全局技能目录 `native-skills/`
- Git 仓库里的 checkpoint 对象
- 窗口尺寸等本地 UI 状态

恢复时若备份版本高于应用支持的最新迁移，会拒绝导入。导入失败会尝试滚回导入前自动备份。

SSH、渠道、工作区 CRUD 已注册。会话按渠道 + 模型启动，不再有档案表。

## 本地查看

```bash
sqlite3 "$HOME/Library/Application Support/com.wenyuan.noxcode/noxcode.db" ".tables"
sqlite3 "$HOME/Library/Application Support/com.wenyuan.noxcode/noxcode.db" \
  "SELECT version, description, success FROM _sqlx_migrations;"
```

应看到 9 张业务表加 `_sqlx_migrations`，且 version 1 成功。
