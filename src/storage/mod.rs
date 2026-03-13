pub mod chat_message;
pub mod node_model;
pub mod ref_model;
pub mod repo_model;

use anyhow::{anyhow, Result};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::OnceCell;

use crate::identity::keypair::KeyPair;

/// 默认根目录：`~/.megaengine`，可由 `MEGAENGINE_ROOT` 环境变量覆盖
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MEGAENGINE_ROOT") {
        return PathBuf::from(dir);
    }

    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".megaengine");
        return p;
    }

    // Windows fallback
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let mut p = PathBuf::from(profile);
        p.push(".megaengine");
        return p;
    }

    // As a last resort fall back to cwd/.megaengine
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push(".megaengine");
    p
}

/// keypair 存放到根目录下
pub fn keypair_path() -> PathBuf {
    let mut p = data_dir();
    fs::create_dir_all(&p).ok();
    p.push("keypair.json");
    p
}

/// 证书路径（默认放到根目录）
pub fn cert_path() -> PathBuf {
    let mut p = data_dir();
    fs::create_dir_all(&p).ok();
    p.push("cert.pem");
    p
}

pub fn key_path() -> PathBuf {
    let mut p = data_dir();
    fs::create_dir_all(&p).ok();
    p.push("key.pem");
    p
}

pub fn ca_cert_path() -> PathBuf {
    let mut p = data_dir();
    fs::create_dir_all(&p).ok();
    p.push("ca-cert.pem");
    p
}

/// SQLite DB 路径
pub fn db_path() -> PathBuf {
    let mut p = data_dir();
    fs::create_dir_all(&p).ok();
    p.push("megaengine.db");
    p
}

async fn execute_sql_ignore_duplicate_column(db: &DatabaseConnection, sql: &str) -> Result<()> {
    match db.execute_unprepared(sql).await {
        Ok(_) => Ok(()),
        Err(err) => {
            let msg = err.to_string().to_lowercase();
            if msg.contains("duplicate column name") {
                return Ok(());
            }
            Err(err.into())
        }
    }
}

fn escape_sqlite_literal(value: &str) -> String {
    value.replace('\'', "''")
}

async fn sqlite_query_one_i64(db: &DatabaseConnection, sql: String) -> Result<i64> {
    let row = db
        .query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await?
        .ok_or_else(|| anyhow!("sqlite query returned no rows"))?;
    Ok(row.try_get_by_index(0)?)
}

async fn sqlite_query_one_string_opt(
    db: &DatabaseConnection,
    sql: String,
) -> Result<Option<String>> {
    let Some(row) = db
        .query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await?
    else {
        return Ok(None);
    };

    let value: Option<String> = row.try_get_by_index(0)?;
    Ok(value)
}

async fn sqlite_has_column(db: &DatabaseConnection, table: &str, column: &str) -> Result<bool> {
    let sql = format!(
        "SELECT COUNT(*) FROM pragma_table_info('{}') WHERE name = '{}'",
        escape_sqlite_literal(table),
        escape_sqlite_literal(column)
    );
    Ok(sqlite_query_one_i64(db, sql).await? > 0)
}

async fn migrate_repos_table(db: &DatabaseConnection) -> Result<()> {
    execute_sql_ignore_duplicate_column(
        db,
        "ALTER TABLE repos ADD COLUMN language TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    execute_sql_ignore_duplicate_column(
        db,
        "ALTER TABLE repos ADD COLUMN size INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    execute_sql_ignore_duplicate_column(
        db,
        "ALTER TABLE repos ADD COLUMN latest_commit_at INTEGER NOT NULL DEFAULT 0",
    )
    .await?;

    if repos_table_needs_rebuild(db).await? {
        rebuild_repos_table(db).await?;
    }

    Ok(())
}

async fn repos_table_needs_rebuild(db: &DatabaseConnection) -> Result<bool> {
    // Legacy schema had a `timestamp` column that can block inserts now that
    // repo writes no longer set it. Rebuild to canonical schema when present.
    sqlite_has_column(db, "repos", "timestamp").await
}

async fn rebuild_repos_table(db: &DatabaseConnection) -> Result<()> {
    let has_language = sqlite_has_column(db, "repos", "language").await?;
    let has_size = sqlite_has_column(db, "repos", "size").await?;
    let has_latest_commit_at = sqlite_has_column(db, "repos", "latest_commit_at").await?;
    let has_bundle = sqlite_has_column(db, "repos", "bundle").await?;
    let has_is_external = sqlite_has_column(db, "repos", "is_external").await?;
    let has_created_at = sqlite_has_column(db, "repos", "created_at").await?;
    let has_updated_at = sqlite_has_column(db, "repos", "updated_at").await?;
    let has_timestamp = sqlite_has_column(db, "repos", "timestamp").await?;

    let now_expr = "CAST(strftime('%s','now') AS INTEGER)";

    let language_expr = if has_language {
        "COALESCE(language, '')"
    } else {
        "''"
    };

    let size_expr = if has_size { "COALESCE(size, 0)" } else { "0" };

    let latest_commit_expr = if has_latest_commit_at {
        "COALESCE(latest_commit_at, 0)"
    } else if has_timestamp {
        "COALESCE(timestamp, 0)"
    } else {
        "0"
    };

    let bundle_expr = if has_bundle {
        "COALESCE(bundle, '')"
    } else {
        "''"
    };

    let is_external_expr = if has_is_external {
        "COALESCE(is_external, 0)"
    } else {
        "0"
    };

    let created_expr = if has_created_at {
        format!("COALESCE(created_at, {now_expr})")
    } else if has_timestamp {
        format!("COALESCE(timestamp, {now_expr})")
    } else {
        now_expr.to_string()
    };

    let updated_expr = if has_updated_at {
        format!("COALESCE(updated_at, {now_expr})")
    } else if has_created_at {
        format!("COALESCE(created_at, {now_expr})")
    } else if has_timestamp {
        format!("COALESCE(timestamp, {now_expr})")
    } else {
        now_expr.to_string()
    };

    let create_sql = "\
        CREATE TABLE repos_new (\
            id TEXT PRIMARY KEY,\
            name TEXT NOT NULL,\
            creator TEXT NOT NULL,\
            description TEXT NOT NULL,\
            language TEXT NOT NULL DEFAULT '',\
            size INTEGER NOT NULL DEFAULT 0,\
            latest_commit_at INTEGER NOT NULL DEFAULT 0,\
            path TEXT NOT NULL,\
            bundle TEXT NOT NULL DEFAULT '',\
            is_external INTEGER NOT NULL DEFAULT 0,\
            created_at INTEGER NOT NULL,\
            updated_at INTEGER NOT NULL\
        );";

    let insert_sql = format!(
        "\
        INSERT OR REPLACE INTO repos_new (\
            id,\
            name,\
            creator,\
            description,\
            language,\
            size,\
            latest_commit_at,\
            path,\
            bundle,\
            is_external,\
            created_at,\
            updated_at\
        )\
        SELECT\
            id,\
            name,\
            creator,\
            description,\
            {language_expr},\
            {size_expr},\
            {latest_commit_expr},\
            path,\
            {bundle_expr},\
            {is_external_expr},\
            {created_expr},\
            {updated_expr}\
        FROM repos\
        WHERE id IS NOT NULL\
          AND name IS NOT NULL\
          AND creator IS NOT NULL\
          AND description IS NOT NULL\
          AND path IS NOT NULL;"
    );

    let drop_sql = "DROP TABLE repos;";
    let rename_sql = "ALTER TABLE repos_new RENAME TO repos;";

    let txn = db.begin().await?;

    txn.execute_unprepared(create_sql).await?;
    txn.execute_unprepared(&insert_sql).await?;
    txn.execute_unprepared(drop_sql).await?;
    txn.execute_unprepared(rename_sql).await?;

    txn.commit().await?;

    Ok(())
}

async fn refs_table_needs_rebuild(db: &DatabaseConnection) -> Result<bool> {
    let pk_sql = format!(
        "SELECT group_concat(name, ',') FROM (\
         SELECT name FROM pragma_table_info('{}') WHERE pk > 0 ORDER BY pk\
         )",
        escape_sqlite_literal("refs")
    );
    let pk = sqlite_query_one_string_opt(db, pk_sql).await?;
    let has_created_at = sqlite_has_column(db, "refs", "created_at").await?;
    Ok(pk.as_deref() != Some("repo_id,ref_name") || !has_created_at)
}

async fn rebuild_refs_table(db: &DatabaseConnection) -> Result<()> {
    let refs_has_created_at = sqlite_has_column(db, "refs", "created_at").await?;
    let refs_has_updated_at = sqlite_has_column(db, "refs", "updated_at").await?;

    let created_expr = if refs_has_created_at {
        "COALESCE(created_at, CAST(strftime('%s','now') AS INTEGER))"
    } else if refs_has_updated_at {
        "COALESCE(updated_at, CAST(strftime('%s','now') AS INTEGER))"
    } else {
        "CAST(strftime('%s','now') AS INTEGER)"
    };

    let updated_expr = if refs_has_updated_at {
        "COALESCE(updated_at, CAST(strftime('%s','now') AS INTEGER))"
    } else if refs_has_created_at {
        "COALESCE(created_at, CAST(strftime('%s','now') AS INTEGER))"
    } else {
        "CAST(strftime('%s','now') AS INTEGER)"
    };

    // Execute the migration statements one-by-one within a transaction
    let txn = db.begin().await?;

    txn.execute(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE TABLE refs_new (
            repo_id TEXT NOT NULL,
            ref_name TEXT NOT NULL,
            commit_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (repo_id, ref_name)
        )"
        .to_owned(),
    ))
    .await?;

    let insert_sql = format!(
        "INSERT OR REPLACE INTO refs_new (repo_id, ref_name, commit_hash, created_at, updated_at)
         SELECT repo_id, ref_name, commit_hash, {created_expr}, {updated_expr}
         FROM refs
         WHERE repo_id IS NOT NULL AND ref_name IS NOT NULL"
    );

    txn.execute(Statement::from_string(
        DbBackend::Sqlite,
        insert_sql,
    ))
    .await?;

    txn.execute(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TABLE refs".to_owned(),
    ))
    .await?;

    txn.execute(Statement::from_string(
        DbBackend::Sqlite,
        "ALTER TABLE refs_new RENAME TO refs".to_owned(),
    ))
    .await?;

    txn.commit().await?;

    Ok(())
}

async fn migrate_refs_table(db: &DatabaseConnection) -> Result<()> {
    if !refs_table_needs_rebuild(db).await? {
        return Ok(());
    }
    rebuild_refs_table(db).await
}

async fn ensure_schema(db: &DatabaseConnection) -> Result<()> {
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS repos (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            creator TEXT NOT NULL,
            description TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT '',
            size INTEGER NOT NULL DEFAULT 0,
            latest_commit_at INTEGER NOT NULL DEFAULT 0,
            path TEXT NOT NULL,
            bundle TEXT NOT NULL DEFAULT '',
            is_external INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .await?;

    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            alias TEXT NOT NULL,
            addresses TEXT NOT NULL,
            node_type INTEGER NOT NULL,
            version INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .await?;

    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS refs (
            repo_id TEXT NOT NULL,
            ref_name TEXT NOT NULL,
            commit_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (repo_id, ref_name)
        )",
    )
    .await?;

    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            \"from\" TEXT NOT NULL,
            \"to\" TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL
        )",
    )
    .await?;

    migrate_repos_table(db).await?;
    migrate_refs_table(db).await?;

    // Align old refs rows that may have default timestamps after ALTER/rebuild.
    db.execute_unprepared(
        "UPDATE refs
         SET created_at = updated_at
         WHERE created_at = 0 AND updated_at > 0",
    )
    .await?;

    Ok(())
}

/// 初始化数据库连接并创建表
pub async fn get_db_conn() -> Result<DatabaseConnection> {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    static DB_POOL: OnceCell<Mutex<HashMap<PathBuf, Arc<OnceCell<DatabaseConnection>>>>> =
        OnceCell::const_new();

    let pool = DB_POOL
        .get_or_init(|| async { Mutex::new(HashMap::new()) })
        .await;

    let path = db_path();

    // 为每个数据库路径维护一个独立的初始化单元，确保每个路径的初始化只执行一次
    let cell = {
        let mut map = pool.lock().await;
        map.entry(path.clone())
            .or_insert_with(|| Arc::new(OnceCell::const_new()))
            .clone()
    };

    // 延迟初始化并缓存全局连接（仅第一次会执行创建表操作）
    let db = cell
        .get_or_try_init(|| {
            let db_path = path.clone();
            async move {
                // 确保目录存在
                if let Some(parent) = db_path.parent() {
                    fs::create_dir_all(parent).ok();
                }

                // 使用合适的 SQLite URL 格式
                let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

                let mut opt = ConnectOptions::new(db_url);
                opt.max_connections(8)
                    .min_connections(1)
                    .connect_timeout(Duration::from_secs(8))
                    .idle_timeout(Duration::from_secs(8))
                    .sqlx_logging(false);

                let db = Database::connect(opt).await?;

                // 运行迁移/建表，兼容已有数据库结构升级
                ensure_schema(&db).await?;

                Ok::<DatabaseConnection, anyhow::Error>(db)
            }
        })
        .await?
        .clone();

    Ok(db)
}

/// 保存密钥对到文件（JSON）
pub fn save_keypair(kp: &KeyPair) -> Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let path = keypair_path();
    let s = serde_json::to_string_pretty(kp)?;
    fs::write(path, s)?;
    Ok(())
}

/// 从文件加载密钥对
pub fn load_keypair() -> Result<KeyPair> {
    let path = keypair_path();
    let s = fs::read_to_string(path)?;
    let kp: KeyPair = serde_json::from_str(&s)?;
    Ok(kp)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_data_dir() {
        let dir = data_dir();
        assert!(dir.ends_with(".megaengine"));
    }

    #[test]
    fn test_keypair_path() {
        let path = keypair_path();
        assert!(path.to_string_lossy().contains("keypair.json"));
    }

    #[test]
    fn test_save_and_load_keypair() -> Result<()> {
        let kp = KeyPair::generate()?;
        save_keypair(&kp)?;

        let loaded = load_keypair()?;
        assert_eq!(
            kp.verifying_key_bytes(),
            loaded.verifying_key_bytes(),
            "Loaded keypair should match saved keypair"
        );

        Ok(())
    }
}
