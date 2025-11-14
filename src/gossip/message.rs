use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, time};

use crate::{
    node::{
        node::{Node, NodeType},
        node_id::NodeId,
    },
    repo::repo_id::RepoId,
    util::timestamp_now,
};

/// Gossip 消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    /// 节点公告
    NodeAnnouncement(NodeAnnouncement),
    /// 仓库公告 (库存公告)
    RepoAnnouncement(RepoAnnouncement),
}

/// 节点公告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAnnouncement {
    pub node_id: NodeId,
    pub version: u8,
    pub alias: String,
    pub node_type: NodeType,
    pub addresses: Vec<SocketAddr>,
}

impl From<Node> for NodeAnnouncement {
    fn from(node: Node) -> Self {
        Self {
            node_id: node.node_id().clone(),
            version: node.version(),
            alias: node.alias().to_string(),
            node_type: node.node_type(),
            addresses: node.addresses().to_vec(),
        }
    }
}

/// 仓库公告- 表示某个节点拥有的仓库列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAnnouncement {
    pub node_id: NodeId,
    pub repos: Vec<RepoId>,
}

/// 带签名的消息包装
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedMessage {
    pub node_id: NodeId,
    pub message: GossipMessage,
    pub timestamp: u64,
    pub signature: String,
}

#[allow(dead_code)]
impl SignedMessage {
    pub fn new_node_sign_message(node: Node) -> Result<Self> {
        let message = GossipMessage::NodeAnnouncement(node.clone().into());

        let mut sign_message = SignedMessage {
            node_id: node.node_id().clone(),
            message,
            timestamp: timestamp_now(),
            signature: "".to_string(),
        };
        let self_hash = sign_message.self_hash();
        let sign = node.sign_message(self_hash.as_slice())?;
        sign_message.signature = hex::encode(sign);
        Ok(sign_message)
    }

    pub fn new_repo_sign_message(repos: Vec<RepoId>, node: Node) -> Result<Self> {
        let message = GossipMessage::RepoAnnouncement(RepoAnnouncement {
            node_id: node.node_id().clone(),
            repos: repos,
        });

        let mut sign_message = SignedMessage {
            node_id: node.node_id().clone(),
            message,
            timestamp: timestamp_now(),
            signature: "".to_string(),
        };
        let self_hash = sign_message.self_hash();
        let sign = node.sign_message(self_hash.as_slice())?;
        sign_message.signature = hex::encode(sign);
        Ok(sign_message)
    }

    pub fn self_hash(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        let message_bytes = serde_json::to_vec(&self.message).unwrap_or_default();

        hasher.update(self.node_id.0.as_bytes());
        hasher.update(&message_bytes);
        hasher.finalize().to_vec()
    }

    /// 获取消息的时间戳
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// 获取消息类型
    pub fn message_type(&self) -> &'static str {
        self.message.message_type()
    }
}

impl GossipMessage {
    /// 获取消息类型
    pub fn message_type(&self) -> &'static str {
        match self {
            GossipMessage::NodeAnnouncement(_) => "node_announcement",
            GossipMessage::RepoAnnouncement(_) => "inventory_announcement",
        }
    }

    /// 获取发送者 NodeId
    pub fn sender(&self) -> &NodeId {
        match self {
            GossipMessage::NodeAnnouncement(na) => &na.node_id,
            GossipMessage::RepoAnnouncement(ra) => &ra.node_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keypair::KeyPair;
    use crate::node::node::Node;

    fn make_node() -> Node {
        let keypair = KeyPair::generate().expect("generate keypair");
        let addresses = vec!["127.0.0.1:8080".parse().unwrap()];
        Node::from_keypair(
            &keypair,
            "test-node",
            addresses,
            crate::node::node::NodeType::Normal,
        )
    }

    #[test]
    fn test_new_node_sign_message() {
        let node = make_node();
        let signed = SignedMessage::new_node_sign_message(node.clone()).expect("sign node message");

        assert_eq!(signed.message_type(), "node_announcement");
        assert!(signed.timestamp() > 0);

        // signature should be a hex string that decodes to 64 bytes (ed25519)
        let sig = hex::decode(&signed.signature).expect("decode hex");
        assert_eq!(sig.len(), 64);

        // self_hash is 32 bytes
        let h = signed.self_hash();
        assert_eq!(h.len(), 32);
    }

    #[test]
    fn test_new_repo_sign_message() {
        let keypair = KeyPair::generate().expect("generate keypair");
        let node = Node::from_keypair(
            &keypair,
            "repo-node",
            vec!["127.0.0.1:9090".parse().unwrap()],
            crate::node::node::NodeType::Relay,
        );

        // generate a repo id
        let repo_id = crate::repo::repo_id::RepoId::generate(
            b"root_commit",
            node_keypair_bytes(&keypair).as_slice(),
        )
        .expect("generate repo id");

        let signed = SignedMessage::new_repo_sign_message(vec![repo_id.clone()], node.clone())
            .expect("sign repo message");

        assert_eq!(signed.message_type(), "inventory_announcement");
        let sig = hex::decode(&signed.signature).expect("decode hex");
        assert_eq!(sig.len(), 64);

        // ensure the embedded repo id is present in message
        if let GossipMessage::RepoAnnouncement(ra) = signed.message {
            assert!(ra.repos.iter().any(|r| r.as_str() == repo_id.as_str()));
        } else {
            panic!("expected RepoAnnouncement");
        }
    }

    fn node_keypair_bytes(kp: &KeyPair) -> Vec<u8> {
        kp.verifying_key.as_bytes().to_vec()
    }
}
