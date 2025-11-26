use crate::identity::keypair::KeyPair;
use anyhow::anyhow;
use anyhow::Result;
use multibase::Base;
use multibase::{decode, encode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct NodeId(pub String);

const DID_KEY_PREFIX: &str = "did:key:";

impl NodeId {
    pub fn from_keypair(keypair: &KeyPair) -> Self {
        let mut prefixed = vec![0xed];
        prefixed.extend_from_slice(keypair.verifying_key_bytes().as_slice());
        NodeId(format!(
            "{}{}",
            DID_KEY_PREFIX,
            encode(Base::Base58Btc, prefixed)
        ))
    }

    pub fn from_string(node_id: &str) -> Result<Self> {
        if !node_id.starts_with(DID_KEY_PREFIX) {
            return Err(anyhow!("invalid NodeId prefix"));
        }

        let encoded = &node_id[DID_KEY_PREFIX.len()..];
        if encoded.is_empty() {
            return Err(anyhow!("empty encoded part"));
        }

        let (base, data) = decode(encoded).map_err(|e| anyhow!("nodeId decode failed: {}", e))?;

        if base != Base::Base58Btc {
            return Err(anyhow!("invalid base format"));
        }

        if data.is_empty() || data[0] != 0xed {
            return Err(anyhow!("invalid key prefix"));
        }

        Ok(NodeId(node_id.to_string()))
    }

    pub fn to_keypair(&self) -> Result<KeyPair> {
        if !self.0.starts_with(DID_KEY_PREFIX) {
            return Err(anyhow!("invalid NodeId prefix"));
        }

        let encoded = &self.0[DID_KEY_PREFIX.len()..];
        let (base, data) = decode(encoded).map_err(|e| anyhow!("nodeId decode failed: {}", e))?;
        if base != Base::Base58Btc {
            return Err(anyhow!("invalid base format"));
        }

        if data.is_empty() || data[0] != 0xed {
            return Err(anyhow!("invalid key prefix"));
        }

        let pubkey_bytes = &data[1..];
        if pubkey_bytes.len() != 32 {
            return Err(anyhow!("invalid key length"));
        }
        let keypair = KeyPair::from_verifying_key_bytes(<[u8; 32]>::try_from(pubkey_bytes)?)?;
        Ok(keypair)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn as_vec(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseNodeIdError;

impl FromStr for NodeId {
    type Err = ParseNodeIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        NodeId::from_string(s).map_err(|_| ParseNodeIdError {})
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_id_from_and_to_keypair() -> Result<()> {
        let kp = KeyPair::generate()?;

        let node_id = NodeId::from_keypair(&kp);
        assert!(node_id.0.starts_with(DID_KEY_PREFIX));
        assert!(node_id.0.len() > 20);

        let recovered = node_id.clone().to_keypair()?;
        assert_eq!(
            kp.verifying_key_bytes(),
            recovered.verifying_key_bytes(),
            "Recovered pubkey should match original"
        );

        Ok(())
    }

    #[test]
    fn test_valid_from_string() -> Result<()> {
        let node_id_str = "did:key:z2DXbAovGq5vNKpXVFyrhVLppMdUCmV1hCNjbUydLMEWasE";
        let node_id = NodeId::from_string(&node_id_str)?;
        assert_eq!(node_id.0, node_id_str);
        Ok(())
    }

    #[test]
    fn test_invalid_prefix_from_string() {
        let result = NodeId::from_string("invalid:zabcdef");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_base_from_string() {
        let encoded = encode(Base::Base64, vec![0xed, 1, 2, 3]);
        let node_id_str = format!("{}{}", DID_KEY_PREFIX, encoded);
        let result = NodeId::from_string(&node_id_str);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("invalid base format"));
    }

    #[test]
    fn test_invalid_key_prefix_from_string() {
        // 构造前缀错误
        let encoded = encode(Base::Base58Btc, vec![0xaa, 1, 2, 3]);
        let node_id_str = format!("{}{}", DID_KEY_PREFIX, encoded);
        let result = NodeId::from_string(&node_id_str);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("invalid key prefix"));
    }

    #[test]
    fn test_invalid_prefix() {
        let node_id = NodeId("invalid:zabc".into());
        let result = node_id.to_keypair();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_encoding() {
        let node_id = NodeId(format!("{}{}", DID_KEY_PREFIX, "notbase58!"));
        let result = node_id.to_keypair();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_key_prefix() {
        let bad_data = encode(Base::Base58Btc, vec![0xaa, 1, 2, 3]);
        let node_id = NodeId(format!("{}{}", DID_KEY_PREFIX, bad_data));
        let result = node_id.to_keypair();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_key_length() {
        let mut data = vec![0xed];
        data.extend_from_slice(&[1, 2, 3, 4, 5]);
        let bad_data = encode(Base::Base58Btc, data);
        let node_id = NodeId(format!("{}{}", DID_KEY_PREFIX, bad_data));
        let result = node_id.to_keypair();
        assert!(result.is_err());
    }
}
