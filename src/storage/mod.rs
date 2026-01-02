pub mod node_model;
pub mod repo_model;
pub mod ref_model;

use anyhow::Result;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
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

/// 初始化数据库连接并创建表
pub async fn init_db() -> Result<DatabaseConnection> {
    static DB: OnceCell<DatabaseConnection> = OnceCell::const_new();

    // 如果已经初始化，直接返回 clone
    if let Some(db) = DB.get() {
        return Ok(db.clone());
    }

    // 延迟初始化并缓存全局连接（仅第一次会执行创建表操作）
    let db_conn = DB
        .get_or_init(|| async {
            let db_path = db_path();

            // 确保目录存在
            if let Some(parent) = db_path.parent() {
                fs::create_dir_all(parent).ok();
            }

            // 使用合适的 SQLite URL 格式
            let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

            let mut opt = ConnectOptions::new(db_url);
            opt.max_connections(5)
                .min_connections(1)
                .connect_timeout(Duration::from_secs(8));

            let db = Database::connect(opt)
                .await
                .expect("failed to connect to db");

            // 运行迁移或创建表（只在初始化时执行）
            let _ = db
                .execute_unprepared(
                    "CREATE TABLE IF NOT EXISTS repos (
                        id TEXT PRIMARY KEY,
                        name TEXT NOT NULL,
                        creator TEXT NOT NULL,
                        description TEXT NOT NULL,
                        timestamp INTEGER NOT NULL,
                        path TEXT NOT NULL,
                        bundle TEXT NOT NULL DEFAULT '',
                        is_external INTEGER NOT NULL DEFAULT 0,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    )",
                )
                .await;

            // 节点表
            let _ = db
                .execute_unprepared(
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
                .await;

            // Refs 表：存储分支和标签的最新 commit
            let _ = db
                .execute_unprepared(
                    "CREATE TABLE IF NOT EXISTS refs (
                        id TEXT PRIMARY KEY,
                        repo_id TEXT NOT NULL,
                        ref_name TEXT NOT NULL,
                        commit_hash TEXT NOT NULL,
                        updated_at INTEGER NOT NULL,
                        UNIQUE(repo_id, ref_name)
                    )",
                )
                .await;

            db
        })
        .await;

    Ok(db_conn.clone())
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
