use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// P2P 仓库描述
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct P2PDescription {
    pub creator: String,
    pub name: String,
    pub description: String,
    pub timestamp: i64,
}

/// P2P 仓库
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repo {
    pub repo_id: String,
    pub refs: HashMap<String, String>,
    pub p2p_description: P2PDescription,
    pub path: PathBuf,
    pub is_external: bool,
    pub bundle: PathBuf,
}

impl Repo {
    /// 创建新仓库
    pub fn new(repo_id: String, p2p_description: P2PDescription, path: PathBuf) -> Self {
        Repo {
            repo_id,
            refs: HashMap::new(),
            p2p_description,
            path,
            is_external: false,
            bundle: PathBuf::new(),
        }
    }

    /// 添加 ref
    pub fn add_ref(&mut self, ref_name: String, commit_hash: String) {
        self.refs.insert(ref_name, commit_hash);
    }

    /// 获取 ref
    pub fn get_ref(&self, ref_name: &str) -> Option<&String> {
        self.refs.get(ref_name)
    }

    /// 更新 ref
    pub fn update_ref(&mut self, ref_name: String, commit_hash: String) -> bool {
        self.refs.insert(ref_name, commit_hash).is_some()
    }

    /// 删除 ref
    pub fn remove_ref(&mut self, ref_name: &str) -> Option<String> {
        self.refs.remove(ref_name)
    }

    /// 获取所有 refs
    pub fn list_refs(&self) -> Vec<(String, String)> {
        self.refs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// 获取仓库地址（P2P 格式）
    pub fn p2p_address(&self) -> String {
        format!("git+p2p://{}", self.repo_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_creation() {
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

        assert_eq!(repo.repo_id, "did:repo:test");
        assert_eq!(repo.p2p_address(), "git+p2p://did:repo:test");
    }

    #[test]
    fn test_repo_refs() {
        let desc = P2PDescription {
            creator: "did:key:test".to_string(),
            name: "test-repo".to_string(),
            description: "A test repository".to_string(),
            timestamp: 1000,
        };

        let mut repo = Repo::new(
            "did:repo:test".to_string(),
            desc,
            PathBuf::from("/tmp/test-repo"),
        );

        repo.add_ref("refs/heads/main".to_string(), "commit1".to_string());
        assert_eq!(
            repo.get_ref("refs/heads/main"),
            Some(&"commit1".to_string())
        );
    }
}
