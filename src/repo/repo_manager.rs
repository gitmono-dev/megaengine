use std::path::PathBuf;

use crate::repo::repo::Repo;
use crate::storage::repo_model::{
    delete_repo_from_db, list_repos, load_repo_from_db, save_repo_to_db,
};
use anyhow::Result;

/// 仓库管理器
/// 管理本地仓库和 P2P 仓库的对应关系，并支持数据库持久化
pub struct RepoManager {}

impl RepoManager {
    /// 创建新的仓库管理器
    pub fn new() -> Self {
        RepoManager {}
    }

    /// 注册仓库
    pub async fn register_repo(&mut self, repo: Repo) -> Result<(), String> {
        save_repo_to_db(&repo).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 根据 RepoId 获取仓库
    pub async fn get_repo(&self, repo_id: &str) -> Result<Option<Repo>> {
        let repo = load_repo_from_db(repo_id).await?;
        Ok(repo)
    }

    /// 根据路径获取仓库 ID
    pub async fn get_repo_id_by_path(&self, path: &PathBuf) -> Result<Option<String>> {
        // 回退到数据库查询
        let repos = list_repos().await?;
        for repo in repos {
            if &repo.path == path {
                return Ok(Some(repo.repo_id));
            }
        }
        Ok(None)
    }

    /// 删除仓库
    pub async fn remove_repo(&mut self, repo_id: &str) -> Result<Option<Repo>> {
        // 先从数据库加载 repo，返回给调用方；再删除数据库记录
        if let Some(repo) = load_repo_from_db(repo_id).await? {
            // 删除数据库记录
            delete_repo_from_db(repo_id).await?;

            Ok(Some(repo))
        } else {
            Ok(None)
        }
    }

    /// 列出所有仓库
    pub async fn list_repos(&self) -> Result<Vec<Repo>> {
        let repos = list_repos().await?;
        Ok(repos)
    }

    /// 获取仓库数量
    pub async fn repo_count(&self) -> Result<usize> {
        let repos = list_repos().await?;
        Ok(repos.len())
    }

    /// 更新 Repo 的 refs（会自动持久化到数据库）
    pub async fn update_repo(&mut self, repo: Repo) -> Result<()> {
        if (load_repo_from_db(repo.repo_id.as_str()).await?).is_some() {
            save_repo_to_db(&repo).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Repository {} not found", repo.repo_id))
        }
    }
}

impl Default for RepoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::repo::repo::P2PDescription;

    use super::*;

    #[tokio::test]
    async fn test_repo_manager() -> Result<()> {
        let mut manager = RepoManager::new();

        let repo_id = "did:repo:test";
        let desc = P2PDescription {
            creator: "did:key:test".to_string(),
            name: "test-repo".to_string(),
            description: "A test repository".to_string(),
            language: "Rust".to_string(),
            latest_commit_at: 2000,
            size: 0,
        };

        let repo = Repo::new(repo_id.to_string(), desc, PathBuf::from("/tmp/test-repo"));

        // 清理之前可能存在的测试数据
        let _ = manager.remove_repo(repo_id).await;

        // 注册前，确保 repo 不存在
        let before = manager.get_repo(repo_id).await?;
        assert!(
            before.is_none(),
            "repo should not exist before registration"
        );

        // 注册 repo
        assert!(manager.register_repo(repo).await.is_ok());

        // 验证 repo 已注册
        let loaded = manager.get_repo(repo_id).await?;
        assert!(loaded.is_some(), "repo should exist after registration");

        // 清理测试数据
        manager.remove_repo(repo_id).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_repo_manager_with_persistence() -> Result<()> {
        // 持久化现在为默认行为
        let mut manager = RepoManager::new();

        let repo_id = "did:repo:test-persist";
        let desc = P2PDescription {
            creator: "did:key:test".to_string(),
            name: "test-repo-persist".to_string(),
            description: "A test repository with persistence".to_string(),
            language: "Rust".to_string(),

            latest_commit_at: 2000,
            size: 0,
        };

        let repo = Repo::new(
            repo_id.to_string(),
            desc,
            PathBuf::from("/tmp/test-repo-persist"),
        );

        // 清理之前可能存在的测试数据
        let _ = manager.remove_repo(repo_id).await;

        // 注册前，确保 repo 不存在
        let before = manager.get_repo(repo_id).await?;
        assert!(
            before.is_none(),
            "repo should not exist before registration"
        );

        // 注册 repo
        assert!(manager.register_repo(repo).await.is_ok());

        // 验证 repo 已注册
        let loaded = manager.get_repo(repo_id).await?;
        assert!(loaded.is_some(), "repo should exist after registration");

        // 删除仓库
        let result = manager.remove_repo(repo_id).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());

        // 验证数据库中已删除该 repo
        let loaded_after = manager.get_repo(repo_id).await?;
        assert!(
            loaded_after.is_none(),
            "repo should not exist after deletion"
        );

        Ok(())
    }
}
