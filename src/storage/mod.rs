use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::identity::keypair::KeyPair;

/// 数据目录：cwd/.megaengine
pub fn data_dir() -> PathBuf {
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push(".megaengine");
    p
}

/// 密钥对文件路径
pub fn keypair_path() -> PathBuf {
    let mut p = data_dir();
    p.push("keypair.json");
    p
}

/// 保存密钥对到文件
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

        // cleanup
        let path = keypair_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
