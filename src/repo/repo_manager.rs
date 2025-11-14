use std::collections::HashMap;
use std::path::PathBuf;

use crate::repo::repo::Repo;

/// 仓库管理器
/// 管理本地仓库和 P2P 仓库的对应关系
pub struct RepoManager {
    // RepoId -> Repo 映射
    repos: HashMap<String, Repo>,
    // 本地路径 -> RepoId 映射
    path_to_repo_id: HashMap<PathBuf, String>,
}

impl RepoManager {
    /// 创建新的仓库管理器
    pub fn new() -> Self {
        RepoManager {
            repos: HashMap::new(),
            path_to_repo_id: HashMap::new(),
        }
    }

    /// 注册仓库
    pub fn register_repo(&mut self, repo: Repo) -> Result<(), String> {
        let repo_id = repo.repo_id.clone();
        let path = repo.path.clone();

        if self.repos.contains_key(&repo_id) {
            return Err(format!("Repository {} already exists", repo_id));
        }

        self.repos.insert(repo_id.clone(), repo);
        self.path_to_repo_id.insert(path, repo_id);
        Ok(())
    }

    /// 根据 RepoId 获取仓库
    pub fn get_repo(&self, repo_id: &str) -> Option<&Repo> {
        self.repos.get(repo_id)
    }

    /// 根据 RepoId 获取仓库（可变）
    pub fn get_repo_mut(&mut self, repo_id: &str) -> Option<&mut Repo> {
        self.repos.get_mut(repo_id)
    }

    /// 根据路径获取仓库 ID
    pub fn get_repo_id_by_path(&self, path: &PathBuf) -> Option<&String> {
        self.path_to_repo_id.get(path)
    }

    /// 删除仓库
    pub fn remove_repo(&mut self, repo_id: &str) -> Option<Repo> {
        if let Some(repo) = self.repos.remove(repo_id) {
            self.path_to_repo_id.remove(&repo.path);
            Some(repo)
        } else {
            None
        }
    }

    /// 列出所有仓库
    pub fn list_repos(&self) -> Vec<&Repo> {
        self.repos.values().collect()
    }

    /// 获取仓库数量
    pub fn repo_count(&self) -> usize {
        self.repos.len()
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

    #[test]
    fn test_repo_manager() {
        let mut manager = RepoManager::new();

        let desc = P2PDescription {
            creator: "did:key:test".to_string(),
            name: "test-repo".to_string(),
            description: "A test repository".to_string(),
            timestamp: 1000,
        };

        let repo = Repo::new(
            "did:repo:test".to_string(),
            desc,
            PathBuf::from("/tmp/test-repo"),
        );

        assert!(manager.register_repo(repo).is_ok());
        assert_eq!(manager.repo_count(), 1);
        assert!(manager.get_repo("did:repo:test").is_some());
    }
}
