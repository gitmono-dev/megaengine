use crate::identity::keypair::KeyPair;
use crate::node::node_id::NodeId;
use crate::transport::config::QuicConfig;
use crate::transport::quic::ConnectionManager;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeType {
    Normal,
    Relay,
}

/// 节点信息（可序列化的部分）
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeInfo {
    pub node_id: NodeId,
    pub alias: String,
    pub addresses: Vec<SocketAddr>,
    pub node_type: NodeType,
    pub version: u8,
}

/// 运行时节点对象，包含网络管理器
#[derive(Clone)]
pub struct Node {
    pub info: NodeInfo,
    pub connection_manager: Option<Arc<Mutex<ConnectionManager>>>,
    pub keypair: KeyPair,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("info", &self.info)
            .field("connection_manager", &"<ConnectionManager>")
            .finish()
    }
}

impl Node {
    pub fn new(
        node_id: NodeId,
        alias: impl Into<String>,
        addresses: Vec<SocketAddr>,
        node_type: NodeType,
        keypair: KeyPair,
    ) -> Self {
        let info = NodeInfo {
            node_id,
            alias: alias.into(),
            addresses,
            node_type,
            version: 1,
        };
        Self {
            info,
            connection_manager: None,
            keypair,
        }
    }

    pub fn from_keypair(
        keypair: &KeyPair,
        alias: impl Into<String>,
        addresses: Vec<SocketAddr>,
        node_type: NodeType,
    ) -> Self {
        let node_id = NodeId::from_keypair(keypair);
        Self::new(node_id, alias, addresses, node_type, keypair.clone())
    }

    pub fn sign_message(&self, msg: &[u8]) -> Result<Vec<u8>> {
        self.keypair.sign(msg).map(|sig| sig.to_bytes().to_vec())
    }

    /// 启动 QUIC 服务端
    pub async fn start_quic_server(&mut self, config: QuicConfig) -> Result<()> {
        let manager = ConnectionManager::run_server(config).await?;
        self.connection_manager = Some(std::sync::Arc::new(tokio::sync::Mutex::new(manager)));
        Ok(())
    }

    /// 获取节点信息的便捷访问器
    pub fn node_id(&self) -> &NodeId {
        &self.info.node_id
    }

    pub fn alias(&self) -> &str {
        &self.info.alias
    }

    pub fn addresses(&self) -> &[SocketAddr] {
        &self.info.addresses
    }

    pub fn node_type(&self) -> NodeType {
        self.info.node_type.clone()
    }

    pub fn version(&self) -> u8 {
        self.info.version
    }

    pub fn keypair(&self) -> &KeyPair {
        &self.keypair
    }
}

#[derive(Clone, Debug)]
pub struct NodeRouting {
    pub node_id: NodeId,
    pub addresses: Vec<SocketAddr>,
    pub last_seen: SystemTime,
    pub ttl: Duration,
    pub score: f32,
}

impl NodeRouting {
    pub fn new(node_id: NodeId, addresses: Vec<SocketAddr>) -> Self {
        Self {
            node_id,
            addresses,
            last_seen: SystemTime::now(),
            ttl: Duration::from_secs(60 * 60 * 24),
            score: 1.0,
        }
    }

    pub fn refresh(&mut self) {
        self.last_seen = SystemTime::now();
    }

    pub fn expired(&self) -> bool {
        self.last_seen
            .elapsed()
            .map(|e| e > self.ttl)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    fn create_sample_node() -> Node {
        let keypair = &KeyPair::generate().unwrap();
        let node_id = NodeId::from_keypair(keypair);
        let alias = "Test Node";
        let addresses = vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )];
        let node_type = NodeType::Normal;

        Node::new(node_id, alias, addresses, node_type, keypair.clone())
    }

    // Test creation of a Node
    #[test]
    fn test_create_node() {
        let node = create_sample_node();

        // Assert basic properties
        assert_eq!(node.alias(), "Test Node");
        assert_eq!(node.node_type(), NodeType::Normal);
        assert_eq!(node.version(), 1);
        assert_eq!(node.addresses().len(), 1);
        assert_eq!(
            node.addresses()[0],
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080)
        );
    }

    // Test `from_keypair` method for Node
    #[test]
    fn test_node_from_keypair() {
        let keypair = KeyPair::generate().unwrap();
        let alias = "Keypair Node";
        let addresses = vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )];
        let node_type = NodeType::Relay;

        let node = Node::from_keypair(&keypair, alias, addresses, node_type);

        // Assert properties based on keypair
        assert_eq!(node.node_type(), NodeType::Relay);
        assert_eq!(node.alias(), alias);
        assert_eq!(
            node.addresses()[0],
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080)
        );
    }

    // Test `NodeRouting::new` method and expiration logic
    #[test]
    fn test_node_routing() {
        let node_id = NodeId::from_keypair(&KeyPair::generate().unwrap());
        let addresses = vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )];

        let mut node_routing = NodeRouting::new(node_id.clone(), addresses);

        // Assert initial values
        assert_eq!(node_routing.node_id, node_id);
        assert_eq!(node_routing.addresses.len(), 1);
        assert_eq!(
            node_routing.addresses[0],
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080)
        );
        assert_eq!(node_routing.score, 1.0);

        // Test refresh method
        let initial_last_seen = node_routing.last_seen;
        node_routing.refresh();
        assert_ne!(node_routing.last_seen, initial_last_seen); // last_seen should have changed

        // Test expiration logic
        std::thread::sleep(Duration::from_secs(2)); // Sleep for 2 seconds to test expiration
        assert_eq!(node_routing.expired(), false); // Not expired if TTL is 24 hours

        // Manually expire the node and check
        node_routing.ttl = Duration::from_secs(1); // Set TTL to 1 second
        std::thread::sleep(Duration::from_secs(2)); // Sleep for 2 seconds to make the node expire
        assert_eq!(node_routing.expired(), true); // Should be expired now
    }

    // Test the `NodeType` enum
    #[test]
    fn test_node_type_enum() {
        // Normal node
        let node = create_sample_node();
        assert_eq!(node.node_type(), NodeType::Normal);

        // Relay node
        let node = Node::new(
            NodeId::from_keypair(&KeyPair::generate().unwrap()),
            "Relay Node",
            vec![SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                8080,
            )],
            NodeType::Relay,
            KeyPair::generate().unwrap(),
        );
        assert_eq!(node.node_type(), NodeType::Relay);
    }
}
