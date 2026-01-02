use anyhow::anyhow;
use anyhow::Result;
use multibase::{decode, encode, Base};
use multihash::Hash;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct RepoId(pub String);

const REPO_KEY_PREFIX: &str = "did:repo:";

impl RepoId {
    pub fn generate(root_commit: &[u8], creator_public_key: &[u8]) -> Result<Self> {
        let mut data = Vec::new();
        data.extend_from_slice(root_commit);
        data.extend_from_slice(creator_public_key);
        let hash = multihash::encode(Hash::SHA3256, &data)?;
        Ok(RepoId(format!(
            "{}{}",
            REPO_KEY_PREFIX,
            encode(Base::Base58Btc, hash)
        )))
    }

    pub fn parse_from_str(repo_id: &str) -> Result<Self> {
        if !repo_id.starts_with(REPO_KEY_PREFIX) {
            return Err(anyhow!("invalid RepoId prefix"));
        }

        let encoded = &repo_id[REPO_KEY_PREFIX.len()..];
        if encoded.is_empty() {
            return Err(anyhow!("empty encoded part"));
        }

        let (base, data) = decode(encoded).map_err(|e| anyhow!("repoId decode failed: {}", e))?;

        if base != Base::Base58Btc {
            return Err(anyhow!("invalid base format"));
        }

        let _ = multihash::encode(Hash::SHA3256, &data)?;

        Ok(RepoId(repo_id.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseRepoIdError;

impl FromStr for RepoId {
    type Err = ParseRepoIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RepoId::parse_from_str(s).map_err(|_| ParseRepoIdError)
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keypair::KeyPair;
    use anyhow::Result;

    // 测试 RepoId 的生成
    #[test]
    fn test_generate_repo_id() -> Result<()> {
        let root_commit = b"root_commit_data";
        let keypair = KeyPair::generate()?;

        // 生成 RepoId
        let repo_id = RepoId::generate(root_commit, keypair.verifying_key.as_bytes())?;
        println!("Generated RepoId: {}", repo_id);
        // 验证 RepoId 是否以 "did:repo:" 开头
        assert!(repo_id.0.starts_with(REPO_KEY_PREFIX));

        Ok(())
    }

    // 测试 RepoId 的解析
    #[test]
    fn test_from_string_valid() -> Result<()> {
        let root_commit = b"root_commit_data"; // 示例 Git 根提交数据
        let keypair = KeyPair::generate()?;

        // 生成 RepoId
        let repo_id = RepoId::generate(root_commit, keypair.verifying_key.as_bytes())?;

        // 从生成的字符串解析回 RepoId
        let parsed_repo_id = RepoId::parse_from_str(&repo_id.0)?;

        // 验证解析出来的 RepoId 与原始 RepoId 是否相同
        assert_eq!(repo_id, parsed_repo_id);

        Ok(())
    }

    // 测试 RepoId 解析的错误情况：无效的前缀
    #[test]
    fn test_from_string_invalid_prefix() {
        let invalid_repo_id = "invalid:repo_id";
        let result = RepoId::parse_from_str(invalid_repo_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_string_empty_encoded_part() {
        let invalid_repo_id = "did:repo:";
        let result = RepoId::parse_from_str(invalid_repo_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_string_invalid_base_format() {
        let invalid_repo_id = "did:repo:xyz123";
        let result = RepoId::parse_from_str(invalid_repo_id);
        assert!(result.is_err());
    }

    // 测试 RepoId 从字符串解析
    #[test]
    fn test_repo_id_from_str() -> Result<()> {
        let repo_id_str = "did:repo:z5fV2HmRQ3EzYYQ2smU2db1JgeWsxzPfYY9GBR1kFH8S5Zr";
        let repo_id: RepoId = repo_id_str.parse().unwrap();

        // 验证解析出来的 RepoId 是否正确
        assert_eq!(repo_id.to_string(), repo_id_str);

        Ok(())
    }

    // 测试 RepoId 的 Display 格式化输出
    #[test]
    fn test_repo_id_display() {
        let repo_id = RepoId("z5fV2HmRQ3EzYYQ2smU2db1JgeWsxzPfYY9GBR1kFH8S5Zr".to_string());

        // 验证 RepoId 的显示
        assert_eq!(
            format!("{}", repo_id),
            "z5fV2HmRQ3EzYYQ2smU2db1JgeWsxzPfYY9GBR1kFH8S5Zr"
        );
    }
}
