use sqlx::SqlitePool;

use super::migrations::get_all_migrations;

pub(crate) async fn setup_migrated_pool() -> SqlitePool {
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
