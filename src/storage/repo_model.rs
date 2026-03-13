use std::path::PathBuf;

use anyhow::Result;
use sea_orm::entity::prelude::*;
use sea_orm::{Set, Unchanged};

use crate::{repo::repo::Repo, storage::get_db_conn};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "repos")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub name: String,
    pub creator: String,
    pub description: String,
    pub language: String,
    pub path: String,
    pub bundle: String,
    pub is_external: bool,
    pub size: i64,
    pub latest_commit_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// 保存或更新 Repo 到数据库
pub async fn save_repo_to_db(repo: &Repo) -> Result<()> {
    let db = get_db_conn().await?;
    let now = chrono::Local::now().timestamp();

    // 查询是否已存在
    let existing = Entity::find_by_id(repo.repo_id.clone()).one(&db).await?;

    if let Some(existing_model) = existing {
        // 记录已存在，更新
        let active_model = ActiveModel {
            id: Unchanged(repo.repo_id.clone()),
            name: Set(repo.p2p_description.name.clone()),
            creator: Set(repo.p2p_description.creator.clone()),
            description: Set(repo.p2p_description.description.clone()),
            language: Set(repo.p2p_description.language.clone()),
            path: Set(repo.path.to_string_lossy().to_string()),
            bundle: Set(repo.bundle.to_string_lossy().to_string()),
            is_external: Set(repo.is_external),
            size: Set(repo.p2p_description.size as i64),
            latest_commit_at: Set(repo.p2p_description.latest_commit_at),
            created_at: Unchanged(existing_model.created_at),
            updated_at: Set(now),
        };
        Entity::update(active_model).exec(&db).await?;
    } else {
        // 记录不存在，插入
        let active_model = ActiveModel {
            id: Set(repo.repo_id.clone()),
            name: Set(repo.p2p_description.name.clone()),
            creator: Set(repo.p2p_description.creator.clone()),
            description: Set(repo.p2p_description.description.clone()),
            language: Set(repo.p2p_description.language.clone()),
            path: Set(repo.path.to_string_lossy().to_string()),
            bundle: Set(repo.bundle.to_string_lossy().to_string()),
            is_external: Set(repo.is_external),
            size: Set(repo.p2p_description.size as i64),
            latest_commit_at: Set(repo.p2p_description.latest_commit_at),
            created_at: Set(now),
            updated_at: Set(now),
        };
        Entity::insert(active_model).exec(&db).await?;
    }

    // 保存 refs 到 refs 表
    crate::storage::ref_model::batch_save_refs(&repo.repo_id, &repo.refs).await?;

    Ok(())
}

/// 从数据库加载 Repo
pub async fn load_repo_from_db(repo_id: &str) -> Result<Option<Repo>> {
    let db = get_db_conn().await?;

    // 使用 find_by_id 直接查询
    if let Some(model) = Entity::find_by_id(repo_id).one(&db).await? {
        // Load refs from ref_model table
        let refs = crate::storage::ref_model::load_refs_for_repo(&model.id).await?;

        let repo = Repo {
            repo_id: model.id,
            refs,
            p2p_description: crate::repo::repo::P2PDescription {
                creator: model.creator,
                name: model.name,
                description: model.description,
                language: model.language,
                latest_commit_at: model.latest_commit_at,
                size: model.size as u64,
            },
            path: PathBuf::from(model.path),
            bundle: PathBuf::from(model.bundle),
            is_external: model.is_external,
        };
        return Ok(Some(repo));
    }

    Ok(None)
}

/// 删除 Repo 从数据库
pub async fn delete_repo_from_db(repo_id: &str) -> Result<()> {
    let db = get_db_conn().await?;
    Entity::delete_by_id(repo_id).exec(&db).await?;
    // Delete associated refs
    crate::storage::ref_model::delete_refs_for_repo(repo_id).await?;
    Ok(())
}

/// 列出所有 Repos
pub async fn list_repos() -> Result<Vec<Repo>> {
    let db = get_db_conn().await?;
    let models = Entity::find().all(&db).await?;

    let mut repos = Vec::new();
    for model in models {
        // Load refs from ref_model table
        let refs = crate::storage::ref_model::load_refs_for_repo(&model.id).await?;

        repos.push(Repo {
            repo_id: model.id,
            refs,
            p2p_description: crate::repo::repo::P2PDescription {
                creator: model.creator,
                name: model.name,
                description: model.description,
                language: model.language,
                latest_commit_at: model.latest_commit_at,
                size: model.size as u64,
            },
            path: PathBuf::from(model.path),
            bundle: PathBuf::from(model.bundle),
            is_external: model.is_external,
        });
    }
    Ok(repos)
}

/// 更新 Repo 的 bundle 路径
pub async fn update_repo_bundle(repo_id: &str, bundle_path: &str) -> Result<()> {
    let db = get_db_conn().await?;

    // 查询是否存在
    if let Some(model) = Entity::find_by_id(repo_id).one(&db).await? {
        let active_model = ActiveModel {
            id: Unchanged(model.id),
            bundle: Set(bundle_path.to_string()),
            updated_at: Unchanged(model.updated_at),
            // Keep other fields unchanged
            name: Unchanged(model.name),
            creator: Unchanged(model.creator),
            description: Unchanged(model.description),
            language: Unchanged(model.language),
            path: Unchanged(model.path),
            is_external: Unchanged(model.is_external),
            size: Unchanged(model.size),
            latest_commit_at: Unchanged(model.latest_commit_at),
            created_at: Unchanged(model.created_at),
        };
        Entity::update(active_model).exec(&db).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_save_and_load_repo() -> Result<()> {
        // 创建测试 Repo
        let desc = crate::repo::repo::P2PDescription {
            creator: "did:node:test333".to_string(),
            name: "test-repo".to_string(),
            description: "A test repository".to_string(),
            language: "Rust".to_string(),
            latest_commit_at: 1000,
            size: 0,
        };

        let mut repo = Repo::new(
            "did:repo:test333".to_string(),
            desc,
            PathBuf::from("/tmp/test-repo"),
        );
        repo.add_ref("refs/heads/main".to_string(), "abc123".to_string());

        // 保存到数据库
        save_repo_to_db(&repo).await?;

        // 从数据库加载
        let loaded = load_repo_from_db("did:repo:test333").await?;
        assert!(loaded.is_some());

        let loaded_repo = loaded.unwrap();
        assert_eq!(loaded_repo.repo_id, repo.repo_id);
        assert_eq!(loaded_repo.p2p_description.name, repo.p2p_description.name);
        assert_eq!(
            loaded_repo.get_ref("refs/heads/main"),
            Some(&"abc123".to_string())
        );

        // 清理
        delete_repo_from_db("did:repo:test333").await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_list_repos() -> Result<()> {
        // 创建多个测试 Repos
        for i in 0..3 {
            let desc = crate::repo::repo::P2PDescription {
                creator: "did:node:test".to_string(),
                name: format!("test-repo-{}", i),
                description: format!("Test repository {}", i),
                language: "Rust".to_string(),
                latest_commit_at: 1000 + i,
                size: 0,
            };

            let repo = Repo::new(
                format!("did:repo:test-{}", i),
                desc,
                PathBuf::from(format!("/tmp/test-repo-{}", i)),
            );

            save_repo_to_db(&repo).await?;
        }

        // 列出所有 Repos
        let repos = list_repos().await?;
        assert!(repos.len() >= 3);

        // 清理
        for i in 0..3 {
            delete_repo_from_db(&format!("did:repo:test-{}", i)).await?;
        }
        Ok(())
    }
}
