use tauri_plugin_sql::Migration;

pub fn get_all_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "noxcode baseline schema",
            sql: r#"
                CREATE TABLE ssh_configs (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    host TEXT NOT NULL,
                    port INTEGER NOT NULL DEFAULT 22,
                    username TEXT NOT NULL,
                    auth_type TEXT NOT NULL DEFAULT 'key' CHECK (auth_type IN ('key', 'password')),
                    private_key_path TEXT,
                    password_ref TEXT,
                    passphrase_ref TEXT,
                    known_hosts_mode TEXT NOT NULL DEFAULT 'accept-new',
                    last_checked_at TEXT,
                    last_check_status TEXT,
                    last_check_message TEXT,
                    password_probe_checked_at TEXT,
                    password_probe_status TEXT,
                    password_probe_message TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE INDEX idx_ssh_configs_name ON ssh_configs(name);
                CREATE INDEX idx_ssh_configs_host ON ssh_configs(host, username);

                CREATE TRIGGER update_ssh_configs_updated_at AFTER UPDATE ON ssh_configs
                    FOR EACH ROW BEGIN UPDATE ssh_configs SET updated_at = datetime('now') WHERE id = NEW.id; END;

                CREATE TABLE ai_channels (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    protocol TEXT NOT NULL,
                    base_url TEXT NOT NULL,
                    api_key TEXT,
                    extra_headers_json TEXT,
                    models_json TEXT NOT NULL DEFAULT '[]',
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_ai_channels_enabled ON ai_channels(enabled, name);

                CREATE TABLE agent_profiles (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    ai_channel_id TEXT REFERENCES ai_channels(id) ON DELETE RESTRICT,
                    model TEXT NOT NULL,
                    reasoning_effort TEXT NOT NULL DEFAULT 'high',
                    system_prompt TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_agent_profiles_channel ON agent_profiles(ai_channel_id);

                CREATE TABLE workspaces (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    workspace_type TEXT NOT NULL DEFAULT 'local'
                        CHECK (workspace_type IN ('local', 'ssh')),
                    repo_path TEXT,
                    ssh_config_id TEXT REFERENCES ssh_configs(id) ON DELETE RESTRICT,
                    remote_repo_path TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_workspaces_type_updated ON workspaces(workspace_type, updated_at DESC);
                CREATE INDEX idx_workspaces_ssh_config_id ON workspaces(ssh_config_id);

                CREATE TABLE agent_sessions (
                    id TEXT PRIMARY KEY,
                    profile_id TEXT REFERENCES agent_profiles(id) ON DELETE SET NULL,
                    workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
                    working_dir TEXT,
                    execution_target TEXT NOT NULL DEFAULT 'local'
                        CHECK (execution_target IN ('local', 'ssh')),
                    ssh_config_id TEXT REFERENCES ssh_configs(id) ON DELETE SET NULL,
                    target_host_label TEXT,
                    session_kind TEXT NOT NULL DEFAULT 'execution',
                    status TEXT NOT NULL DEFAULT 'pending',
                    started_at TEXT NOT NULL DEFAULT (datetime('now')),
                    ended_at TEXT,
                    exit_code INTEGER,
                    resume_session_id TEXT,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    total_tokens INTEGER,
                    reasoning_tokens INTEGER,
                    cached_tokens INTEGER,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_agent_sessions_profile_started
                    ON agent_sessions(profile_id, started_at DESC);
                CREATE INDEX idx_agent_sessions_workspace_started
                    ON agent_sessions(workspace_id, started_at DESC);
                CREATE INDEX idx_agent_sessions_status
                    ON agent_sessions(status, started_at DESC);
                CREATE INDEX idx_agent_sessions_ssh_config_id
                    ON agent_sessions(ssh_config_id, started_at DESC);

                CREATE TABLE agent_session_events (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
                    event_type TEXT NOT NULL,
                    message TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_agent_session_events_session_created
                    ON agent_session_events(session_id, created_at);
                CREATE INDEX idx_agent_session_events_created_at
                    ON agent_session_events(created_at);

                CREATE TABLE native_api_call_logs (
                    id TEXT PRIMARY KEY,
                    call_id TEXT NOT NULL,
                    attempt INTEGER NOT NULL DEFAULT 1,
                    channel_id TEXT,
                    channel_name TEXT,
                    protocol TEXT NOT NULL,
                    response_encoding TEXT,
                    model TEXT,
                    thinking_enabled INTEGER NOT NULL DEFAULT 0,
                    thinking_level TEXT,
                    request_format TEXT NOT NULL,
                    request_body TEXT,
                    request_truncated INTEGER NOT NULL DEFAULT 0,
                    response_body TEXT,
                    response_truncated INTEGER NOT NULL DEFAULT 0,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    cached_tokens INTEGER,
                    total_tokens INTEGER,
                    first_token_ms INTEGER,
                    duration_ms INTEGER,
                    status TEXT NOT NULL,
                    http_status INTEGER,
                    error_message TEXT,
                    session_id TEXT,
                    profile_id TEXT,
                    workspace_id TEXT,
                    subagent_id TEXT,
                    call_kind TEXT,
                    execution_target TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE INDEX idx_native_api_call_logs_created_at
                    ON native_api_call_logs(created_at DESC);
                CREATE INDEX idx_native_api_call_logs_channel_name
                    ON native_api_call_logs(channel_name);
                CREATE INDEX idx_native_api_call_logs_model
                    ON native_api_call_logs(model);
                CREATE INDEX idx_native_api_call_logs_status
                    ON native_api_call_logs(status);
                CREATE INDEX idx_native_api_call_logs_session_id
                    ON native_api_call_logs(session_id);
                CREATE INDEX idx_native_api_call_logs_call_id
                    ON native_api_call_logs(call_id);
                CREATE INDEX idx_native_api_call_logs_workspace_created
                    ON native_api_call_logs(workspace_id, created_at DESC);
                CREATE INDEX idx_native_api_call_logs_execution_target
                    ON native_api_call_logs(execution_target, created_at DESC);

                CREATE TABLE native_session_transcripts (
                    session_record_id TEXT PRIMARY KEY,
                    profile_id TEXT,
                    workspace_id TEXT,
                    model TEXT NOT NULL,
                    turns INTEGER NOT NULL DEFAULT 0,
                    messages_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    deleted_at TEXT
                );

                CREATE INDEX idx_native_transcripts_workspace
                    ON native_session_transcripts(workspace_id);

                CREATE TABLE git_checkpoints (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
                    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    seq INTEGER NOT NULL,
                    ref_name TEXT NOT NULL,
                    commit_oid TEXT NOT NULL,
                    parent_oid TEXT,
                    label TEXT,
                    kind TEXT NOT NULL DEFAULT 'manual'
                        CHECK (kind IN ('session_start', 'after_tool_call', 'manual', 'auto_pre_restore')),
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE UNIQUE INDEX idx_git_checkpoints_session ON git_checkpoints(session_id, seq);
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "drop agent_profiles; sessions use ai_channel_id",
            sql: r#"
                PRAGMA foreign_keys = OFF;

                CREATE TABLE agent_sessions_new (
                    id TEXT PRIMARY KEY,
                    ai_channel_id TEXT REFERENCES ai_channels(id) ON DELETE SET NULL,
                    workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
                    working_dir TEXT,
                    execution_target TEXT NOT NULL DEFAULT 'local'
                        CHECK (execution_target IN ('local', 'ssh')),
                    ssh_config_id TEXT REFERENCES ssh_configs(id) ON DELETE SET NULL,
                    target_host_label TEXT,
                    session_kind TEXT NOT NULL DEFAULT 'execution',
                    status TEXT NOT NULL DEFAULT 'pending',
                    started_at TEXT NOT NULL DEFAULT (datetime('now')),
                    ended_at TEXT,
                    exit_code INTEGER,
                    resume_session_id TEXT,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    total_tokens INTEGER,
                    reasoning_tokens INTEGER,
                    cached_tokens INTEGER,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                INSERT INTO agent_sessions_new (
                    id, ai_channel_id, workspace_id, working_dir, execution_target,
                    ssh_config_id, target_host_label, session_kind, status,
                    started_at, ended_at, exit_code, resume_session_id,
                    input_tokens, output_tokens, total_tokens, reasoning_tokens,
                    cached_tokens, created_at
                )
                SELECT
                    s.id,
                    p.ai_channel_id,
                    s.workspace_id,
                    s.working_dir,
                    s.execution_target,
                    s.ssh_config_id,
                    s.target_host_label,
                    s.session_kind,
                    s.status,
                    s.started_at,
                    s.ended_at,
                    s.exit_code,
                    s.resume_session_id,
                    s.input_tokens,
                    s.output_tokens,
                    s.total_tokens,
                    s.reasoning_tokens,
                    s.cached_tokens,
                    s.created_at
                FROM agent_sessions s
                LEFT JOIN agent_profiles p ON p.id = s.profile_id;

                DROP TABLE agent_sessions;
                ALTER TABLE agent_sessions_new RENAME TO agent_sessions;

                CREATE INDEX idx_agent_sessions_channel_started
                    ON agent_sessions(ai_channel_id, started_at DESC);
                CREATE INDEX idx_agent_sessions_workspace_started
                    ON agent_sessions(workspace_id, started_at DESC);
                CREATE INDEX idx_agent_sessions_status
                    ON agent_sessions(status, started_at DESC);
                CREATE INDEX idx_agent_sessions_ssh_config_id
                    ON agent_sessions(ssh_config_id, started_at DESC);

                DROP TABLE agent_profiles;

                PRAGMA foreign_keys = ON;
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "agent_sessions.title from first user prompt",
            sql: r#"
                ALTER TABLE agent_sessions ADD COLUMN title TEXT;

                UPDATE agent_sessions
                SET title = (
                    SELECT TRIM(
                        CASE
                            WHEN e.message LIKE '[USER_INPUT]%' THEN substr(e.message, 13)
                            WHEN e.message LIKE '[用户输入]%' THEN substr(e.message, 7)
                            ELSE e.message
                        END
                    )
                    FROM agent_session_events e
                    WHERE e.session_id = agent_sessions.id
                      AND (
                          e.message LIKE '[USER_INPUT]%'
                          OR e.message LIKE '[用户输入]%'
                      )
                    ORDER BY e.created_at ASC
                    LIMIT 1
                )
                WHERE title IS NULL;
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "agent_sessions.pinned for sidebar pin",
            sql: r#"
                ALTER TABLE agent_sessions ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
        Migration {
            version: 5,
            description: "agent_sessions.context_usage_json last context occupancy",
            sql: r#"
                ALTER TABLE agent_sessions ADD COLUMN context_usage_json TEXT;
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
        Migration {
            version: 6,
            description: "ssh_configs configurable algorithms",
            sql: r#"
                ALTER TABLE ssh_configs ADD COLUMN algorithms_json TEXT;
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
        Migration {
            version: 7,
            description: "activity audit logs",
            sql: r#"
                CREATE TABLE activity_logs (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    workspace_id TEXT,
                    session_id TEXT,
                    summary TEXT NOT NULL,
                    payload_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_activity_logs_workspace_created
                    ON activity_logs(workspace_id, created_at DESC);
            "#,
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
    ]
}

pub fn latest_migration_version() -> i64 {
    get_all_migrations()
        .last()
        .map(|migration| migration.version)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use sqlx::{Row, SqlitePool};

    use super::{get_all_migrations, latest_migration_version};

    async fn setup_test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("create sqlite memory pool");

        for migration in get_all_migrations() {
            sqlx::raw_sql(migration.sql)
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("run migration {}: {}", migration.version, error));
        }

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("enable foreign keys");

        pool
    }

    fn table_names_query() -> &'static str {
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    }

    #[test]
    fn migration_versions_are_contiguous() {
        for (index, migration) in get_all_migrations().iter().enumerate() {
            assert_eq!(migration.version, index as i64 + 1);
        }
        assert_eq!(latest_migration_version(), 7);
        assert_eq!(
            get_all_migrations()
                .last()
                .map(|migration| migration.version),
            Some(latest_migration_version())
        );
    }

    #[test]
    fn agent_sessions_has_title_column() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;
            let columns: Vec<String> = sqlx::query(
                "SELECT name FROM pragma_table_info('agent_sessions') WHERE name = 'title'",
            )
            .fetch_all(&pool)
            .await
            .expect("read title column")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
            assert_eq!(columns, vec!["title"]);
        });
    }

    #[test]
    fn agent_sessions_has_pinned_column() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;
            let columns: Vec<String> = sqlx::query(
                "SELECT name FROM pragma_table_info('agent_sessions') WHERE name = 'pinned'",
            )
            .fetch_all(&pool)
            .await
            .expect("read pinned column")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
            assert_eq!(columns, vec!["pinned"]);
        });
    }

    #[test]
    fn agent_sessions_has_context_usage_json_column() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;
            let columns: Vec<String> = sqlx::query(
                "SELECT name FROM pragma_table_info('agent_sessions') WHERE name = 'context_usage_json'",
            )
            .fetch_all(&pool)
            .await
            .expect("read context_usage_json column")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
            assert_eq!(columns, vec!["context_usage_json"]);
        });
    }

    #[test]
    fn latest_schema_has_nine_tables_without_profiles() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;
            let tables: Vec<String> = sqlx::query(table_names_query())
                .fetch_all(&pool)
                .await
                .expect("list tables")
                .into_iter()
                .map(|row| row.get::<String, _>("name"))
                .collect();

            assert_eq!(
                tables,
                vec![
                    "activity_logs",
                    "agent_session_events",
                    "agent_sessions",
                    "ai_channels",
                    "git_checkpoints",
                    "native_api_call_logs",
                    "native_session_transcripts",
                    "ssh_configs",
                    "workspaces",
                ]
            );
        });
    }

    #[test]
    fn ssh_configs_columns_match_codex_ai_v23() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;
            let columns: Vec<String> =
                sqlx::query("SELECT name FROM pragma_table_info('ssh_configs') ORDER BY name")
                    .fetch_all(&pool)
                    .await
                    .expect("read ssh_configs columns")
                    .into_iter()
                    .map(|row| row.get::<String, _>("name"))
                    .collect();

            assert_eq!(
                columns,
                vec![
                    "algorithms_json",
                    "auth_type",
                    "created_at",
                    "host",
                    "id",
                    "known_hosts_mode",
                    "last_check_message",
                    "last_check_status",
                    "last_checked_at",
                    "name",
                    "passphrase_ref",
                    "password_probe_checked_at",
                    "password_probe_message",
                    "password_probe_status",
                    "password_ref",
                    "port",
                    "private_key_path",
                    "updated_at",
                    "username",
                ]
            );
        });
    }

    #[test]
    fn ai_channels_has_api_key_not_api_key_ref() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;
            let columns: Vec<String> = sqlx::query(
                "SELECT name FROM pragma_table_info('ai_channels') WHERE name IN ('id', 'protocol', 'base_url', 'api_key', 'api_key_ref', 'models_json', 'enabled') ORDER BY name",
            )
            .fetch_all(&pool)
            .await
            .expect("read ai_channels columns")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();

            assert_eq!(
                columns,
                vec![
                    "api_key",
                    "base_url",
                    "enabled",
                    "id",
                    "models_json",
                    "protocol"
                ]
            );
        });
    }

    #[test]
    fn foreign_keys_enforce_restrict_and_cascade() {
        tauri::async_runtime::block_on(async {
            let pool = setup_test_pool().await;

            sqlx::query(
                "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-1', 'demo', 'openai', 'https://example.com')",
            )
            .execute(&pool)
            .await
            .expect("insert ai channel");

            sqlx::query(
                "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'local', 'local')",
            )
            .execute(&pool)
            .await
            .expect("insert workspace");

            sqlx::query(
                "INSERT INTO agent_sessions (id, ai_channel_id, workspace_id, status) VALUES ('sess-1', 'ch-1', 'ws-1', 'pending')",
            )
            .execute(&pool)
            .await
            .expect("insert session");

            sqlx::query(
                "INSERT INTO agent_session_events (id, session_id, event_type) VALUES ('evt-1', 'sess-1', 'started')",
            )
            .execute(&pool)
            .await
            .expect("insert session event");

            sqlx::query(
                r#"
                INSERT INTO git_checkpoints (
                    id, session_id, workspace_id, seq, ref_name, commit_oid, kind
                ) VALUES (
                    'cp-1', 'sess-1', 'ws-1', 0, 'refs/noxcode/checkpoints/sess-1/0', 'abc123', 'session_start'
                )
                "#,
            )
            .execute(&pool)
            .await
            .expect("insert checkpoint");

            sqlx::query("DELETE FROM ai_channels WHERE id = 'ch-1'")
                .execute(&pool)
                .await
                .expect("deleting channel should set session channel to null");
            let session_channel: Option<String> =
                sqlx::query_scalar("SELECT ai_channel_id FROM agent_sessions WHERE id = 'sess-1'")
                    .fetch_one(&pool)
                    .await
                    .expect("read session channel");
            assert!(session_channel.is_none());

            sqlx::query("DELETE FROM agent_sessions WHERE id = 'sess-1'")
                .execute(&pool)
                .await
                .expect("delete session");

            let remaining_events: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM agent_session_events")
                    .fetch_one(&pool)
                    .await
                    .expect("count events");
            let remaining_checkpoints: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM git_checkpoints")
                    .fetch_one(&pool)
                    .await
                    .expect("count checkpoints");

            assert_eq!(remaining_events, 0);
            assert_eq!(remaining_checkpoints, 0);
        });
    }
}
