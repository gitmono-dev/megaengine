use crate::node::node_id::NodeId;
use crate::transport::config::QuicConfig;
use anyhow::{Context, Result};
use quinn::{Connection, Endpoint, Incoming};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info};

use std::time::Duration;
use tokio::sync::mpsc::Sender as TokioSender;

const READ_BUF_SIZE: usize = 1024 * 1024;
const CONNECTION_CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

// 消息前缀：用于区分 Gossip 控制消息和数据传输
const GOSSIP_MESSAGE_PREFIX: &[u8] = b"GOSSIP:";
const DATA_MESSAGE_PREFIX: &[u8] = b"DATA:";

// Type alias for Gossip 消息发送端（控制流）
type GossipMessageSender = Arc<Mutex<Option<TokioSender<(NodeId, Vec<u8>)>>>>;
// Type alias for 数据传输发送端（数据流）
type DataMessageSender = Arc<Mutex<Option<TokioSender<(NodeId, Vec<u8>)>>>>;

#[derive(Debug, Clone)]
pub struct ConnectionManager {
    #[allow(dead_code)]
    config: QuicConfig,
    endpoint: Arc<Endpoint>,
    connection_tx: mpsc::Sender<QuicConnection>,
    connections: Arc<Mutex<HashMap<NodeId, Arc<QuicConnection>>>>,
    // 区分 Gossip 消息（控制流）和数据传输流
    gossip_sender: GossipMessageSender,
    data_sender: DataMessageSender,
}

#[derive(Debug, Clone)]
pub struct QuicConnection {
    pub connection: Connection,
    pub peer_addr: SocketAddr,
    pub node_id: NodeId,
    pub connection_type: ConnectionType,
}

#[derive(Debug, Clone)]
pub enum ConnectionType {
    Client,
    Server,
}

impl ConnectionManager {
    fn server(config: QuicConfig) -> Result<(Self, Receiver<QuicConnection>)> {
        let server_config = config.get_server_config()?;

        let mut endpoint = Endpoint::server(server_config, config.bind_addr)
            .context("Failed to create QUIC server endpoint")?;
        let client_config = config.get_client_config()?;
        endpoint.set_default_client_config(client_config);

        info!(
            "The quic service starts on address {}",
            endpoint.local_addr()?
        );

        let (connection_tx, connection_rx) = mpsc::channel(8);

        let transport = Self {
            config,
            endpoint: Arc::new(endpoint),
            connection_tx,
            connections: Arc::new(Mutex::new(HashMap::new())),
            gossip_sender: Arc::new(Mutex::new(None)),
            data_sender: Arc::new(Mutex::new(None)),
        };
        Ok((transport, connection_rx))
    }

    pub async fn run_server(config: QuicConfig) -> Result<Self> {
        let (manager, mut conn_rx) = ConnectionManager::server(config)?;
        let endpoint = Arc::clone(&manager.endpoint);
        let connection_tx = manager.connection_tx.clone();
        let connections = Arc::clone(&manager.connections);
        let manager_clone = manager.clone();

        manager.start_connection_cleanup();

        tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                info!("Accepting connection from {}", incoming.remote_address());
                let tx = connection_tx.clone();
                let manager_clone = manager_clone.clone();
                tokio::spawn(async move {
                    match Self::accept_connection(incoming).await {
                        Ok((conn, msg_rx)) => {
                            if let Err(e) = tx.send(conn.clone()).await {
                                error!("Failed to send connection: {}", e);
                                return;
                            }
                            manager_clone
                                .spawn_message_handler(conn.node_id.clone(), msg_rx)
                                .await;
                        }
                        Err(e) => {
                            error!("Connection failed: {}", e);
                        }
                    }
                });
            }
        });

        // 保存连接
        tokio::spawn(async move {
            while let Some(conn) = conn_rx.recv().await {
                connections
                    .lock()
                    .await
                    .insert(conn.node_id.clone(), Arc::from(conn.clone()));
            }
        });

        Ok(manager.clone())
    }

    pub async fn accept_connection(
        incoming: Incoming,
    ) -> Result<(QuicConnection, Receiver<Vec<u8>>)> {
        let connection = incoming.await?;
        let peer_addr = connection.remote_address();

        // 等待客户端发来的身份流
        let mut recv = connection.accept_uni().await?;
        let node_id_bytes = recv.read_to_end(READ_BUF_SIZE).await?;
        let node_id_str = String::from_utf8(node_id_bytes)?;
        let node_id: NodeId = node_id_str.parse().unwrap();

        info!(
            "Accepted connection from {}, NodeId = {}",
            peer_addr, node_id
        );

        let (message_tx, message_rx) = mpsc::channel(32);
        let connection_clone = connection.clone();
        tokio::spawn(async move {
            while let Ok(mut recv) = connection_clone.accept_uni().await {
                if let Ok(msg) = recv.read_to_end(READ_BUF_SIZE).await {
                    if message_tx.send(msg).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok((
            QuicConnection {
                connection,
                peer_addr,
                node_id,
                connection_type: ConnectionType::Server,
            },
            message_rx,
        ))
    }

    /// 生成消息处理任务，将接收到的消息路由到对应的处理器（Gossip 或数据传输）
    ///
    /// 路由策略基于消息前缀：
    /// - b"GOSSIP:" 前缀：路由到 gossip_sender（控制流消息）
    /// - b"DATA:" 前缀：路由到 data_sender（数据传输）
    /// - 无前缀：默认路由到 gossip_sender（向后兼容）
    async fn spawn_message_handler(&self, peer_id: NodeId, mut receiver: Receiver<Vec<u8>>) {
        let gossip = Arc::clone(&self.gossip_sender);
        let data = Arc::clone(&self.data_sender);

        tokio::spawn(async move {
            while let Some(bytes) = receiver.recv().await {
                // 检查消息前缀来路由
                let is_data_transfer = bytes.starts_with(DATA_MESSAGE_PREFIX);

                if is_data_transfer {
                    // 移除前缀并转发到 data_sender
                    let payload = bytes[DATA_MESSAGE_PREFIX.len()..].to_vec();
                    let maybe_data = data.lock().await;
                    if let Some(tx) = maybe_data.as_ref() {
                        let _ = tx.send((peer_id.clone(), payload)).await;
                        continue;
                    }
                }

                // 检查并移除 GOSSIP 前缀（如果存在）
                let payload = if bytes.starts_with(GOSSIP_MESSAGE_PREFIX) {
                    bytes[GOSSIP_MESSAGE_PREFIX.len()..].to_vec()
                } else {
                    bytes.clone()
                };

                // 路由到 gossip_sender
                let maybe_gossip = gossip.lock().await;
                if let Some(tx) = maybe_gossip.as_ref() {
                    let _ = tx.send((peer_id.clone(), payload)).await;
                } else {
                    let message = String::from_utf8(payload).unwrap_or_default();
                    info!("Received message from {}: {}", peer_id, message);
                }
            }
        });
    }

    /// 注册 Gossip 消息接收器（用于控制流消息）
    pub async fn register_gossip_sender(&self, tx: TokioSender<(NodeId, Vec<u8>)>) {
        let mut guard = self.gossip_sender.lock().await;
        *guard = Some(tx);
    }

    /// 注册数据传输接收器（用于大文件/二进制数据）
    pub async fn register_data_sender(&self, tx: TokioSender<(NodeId, Vec<u8>)>) {
        let mut guard = self.data_sender.lock().await;
        *guard = Some(tx);
    }

    /// Return list of connected peer NodeIds
    pub async fn list_peers(&self) -> Vec<NodeId> {
        let connections = self.connections.lock().await;
        connections.keys().cloned().collect()
    }

    /// Start background task to periodically clean up stale connections
    pub fn start_connection_cleanup(&self) {
        let connections = Arc::clone(&self.connections);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CONNECTION_CLEANUP_INTERVAL);
            loop {
                interval.tick().await;
                let mut conns = connections.lock().await;
                let mut dead_nodes = Vec::new();

                for (node_id, conn) in conns.iter() {
                    if conn.connection.close_reason().is_some() {
                        info!("Connection to node[{}] closed", node_id);
                        dead_nodes.push(node_id.clone());
                    }
                }

                for node_id in dead_nodes {
                    conns.remove(&node_id);
                    info!("Cleaned up stale connection for node: {}", node_id);
                }
            }
        });
    }

    pub async fn connect(
        &self,
        self_node_id: NodeId,
        target_node_id: NodeId,
        addrs: Vec<SocketAddr>,
    ) -> Result<()> {
        let endpoint = self.endpoint.clone();
        let mut connection = None;

        info!("Trying to connect to node[{}]", target_node_id.to_string());
        for addr in addrs.iter() {
            match endpoint.connect(*addr, "localhost")?.await {
                Ok(c) => {
                    connection = Some(c);
                    break;
                }
                Err(_) => continue,
            }
        }

        let connection = match connection {
            Some(c) => c,
            None => {
                return Err(anyhow::anyhow!(
                    "Failed to connect to node[{}], no address available",
                    target_node_id
                ))
            }
        };

        let peer_addr = connection.remote_address();
        info!(
            "Node[{}] connect to[[{}] successfully: {}",
            self_node_id.to_string(),
            target_node_id.to_string(),
            peer_addr
        );

        //Send node_id
        let mut send = connection.open_uni().await?;
        send.write_all(self_node_id.as_bytes()).await?;
        send.finish()?;

        let quic_conn = QuicConnection {
            connection: connection.clone(),
            peer_addr,
            node_id: target_node_id.clone(),
            connection_type: ConnectionType::Client,
        };
        let connections = Arc::clone(&self.connections);
        connections
            .lock()
            .await
            .insert(target_node_id.clone(), Arc::from(quic_conn.clone()));

        // 启动消息接收任务，用于接收服务端发来的消息
        let peer_id = target_node_id.clone();
        let connection_clone = connection.clone();
        let gossip_sender = Arc::clone(&self.gossip_sender);
        let data_sender = Arc::clone(&self.data_sender);

        tokio::spawn(async move {
            while let Ok(mut recv) = connection_clone.accept_uni().await {
                if let Ok(msg) = recv.read_to_end(READ_BUF_SIZE).await {
                    // 基于前缀路由消息
                    let is_data_transfer = msg.starts_with(DATA_MESSAGE_PREFIX);

                    if is_data_transfer {
                        // 移除前缀并路由到 data_sender
                        let payload = msg[DATA_MESSAGE_PREFIX.len()..].to_vec();
                        let maybe_data = data_sender.lock().await;
                        if let Some(tx) = maybe_data.as_ref() {
                            let _ = tx.send((peer_id.clone(), payload)).await;
                            continue;
                        }
                    }

                    // 检查并移除 GOSSIP 前缀（如果存在）
                    let payload = if msg.starts_with(GOSSIP_MESSAGE_PREFIX) {
                        msg[GOSSIP_MESSAGE_PREFIX.len()..].to_vec()
                    } else {
                        msg.clone()
                    };

                    // 路由到 gossip_sender
                    let maybe_gossip = gossip_sender.lock().await;
                    if let Some(tx) = maybe_gossip.as_ref() {
                        let _ = tx.send((peer_id.clone(), payload)).await;
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn send_message(&self, node_id: NodeId, message: Vec<u8>) -> Result<()> {
        let connections = self.connections.lock().await;
        let conn = connections.get(&node_id).with_context(|| {
            format!(
                "Failed to send message to node[{}], connection not found",
                node_id
            )
        })?;

        let mut sender = conn.connection.open_uni().await?;
        sender.write_all(message.as_slice()).await?;
        sender.finish()?;
        Ok(())
    }

    /// 发送 Gossip 消息（会自动添加 GOSSIP: 前缀）
    pub async fn send_gossip_message(&self, node_id: NodeId, message: Vec<u8>) -> Result<()> {
        let mut prefixed = Vec::with_capacity(GOSSIP_MESSAGE_PREFIX.len() + message.len());
        prefixed.extend_from_slice(GOSSIP_MESSAGE_PREFIX);
        prefixed.extend_from_slice(&message);
        self.send_message(node_id, prefixed).await
    }

    /// 发送数据消息（会自动添加 DATA: 前缀，用于大文件传输）
    pub async fn send_data_message(&self, node_id: NodeId, message: Vec<u8>) -> Result<()> {
        let mut prefixed = Vec::with_capacity(DATA_MESSAGE_PREFIX.len() + message.len());
        prefixed.extend_from_slice(DATA_MESSAGE_PREFIX);
        prefixed.extend_from_slice(&message);
        self.send_message(node_id, prefixed).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        identity::keypair::KeyPair,
        node::node::{Node, NodeType},
    };
    use std::sync::Once;
    use tokio::time::Duration;

    static RUSTLS_INIT: Once = Once::new();

    fn init() {
        // Install ring crypto provider only once per test process.
        RUSTLS_INIT.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    // Mock configuration for the tests
    fn mock_quic_config() -> QuicConfig {
        // tracing subscriber may only be initialized once per process; ignore error if already set.
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .try_init();

        // Ensure certificates exist for tests - generate separate cert/key for server 1
        let _ = crate::transport::cert::ensure_certificates(
            "cert/cert.pem",
            "cert/key.pem",
            "cert/ca-cert.pem",
        );

        // Use mock configuration for the tests, ideally mock the actual methods
        QuicConfig::new(
            "0.0.0.0:0".parse().unwrap(),
            "cert/cert.pem".to_string(),
            "cert/key.pem".to_string(),
            "cert/ca-cert.pem".to_string(),
        )
    }

    fn mock_quic_config2() -> QuicConfig {
        // Ensure certificates exist for tests - generate separate cert/key for server 2
        let _ = crate::transport::cert::ensure_certificates(
            "cert/cert2.pem",
            "cert/key2.pem",
            "cert/ca-cert.pem",
        );

        // Use mock configuration for the tests, ideally mock the actual methods
        QuicConfig::new(
            "0.0.0.0:0".parse().unwrap(),
            "cert/cert2.pem".to_string(),
            "cert/key2.pem".to_string(),
            "cert/ca-cert.pem".to_string(),
        )
    }

    // Test the `server` method
    #[tokio::test]
    async fn test_server_creation() {
        init();
        let config = mock_quic_config();

        let manager = ConnectionManager::run_server(config).await;
        assert!(manager.is_ok());

        tokio::time::sleep(Duration::from_millis(500)).await;
        let quic_transport = manager.unwrap();
        assert!(quic_transport.connections.lock().await.is_empty());
    }

    // Test the `connect` method
    #[tokio::test]
    async fn test_client_connection() {
        init();
        let keypair1 = KeyPair::generate().expect("generate keypair");
        let keypair2 = KeyPair::generate().expect("generate keypair");

        let config = mock_quic_config();
        let manager = ConnectionManager::run_server(config).await;
        assert!(manager.is_ok());
        let manager = manager.unwrap();
        // give the server a moment to start and bind
        tokio::time::sleep(Duration::from_millis(200)).await;

        let addr1 = manager.endpoint.local_addr().expect("get local addr");
        let addr1 = format!("127.0.0.1:{}", addr1.port()).parse().unwrap();
        let node1 = Node::new(
            NodeId::from_keypair(&keypair1),
            "",
            vec![addr1],
            NodeType::Normal,
            keypair1.clone(),
        );

        let config2 = mock_quic_config2();
        let manager2 = ConnectionManager::run_server(config2).await;
        assert!(manager2.is_ok());
        let manager2 = manager2.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let addr2 = manager2.endpoint.local_addr().expect("get local addr");
        let addr2 = format!("127.0.0.1:{}", addr2.port()).parse().unwrap();
        let node2 = Node::new(
            NodeId::from_keypair(&keypair2),
            "",
            vec![addr2],
            NodeType::Normal,
            keypair2.clone(),
        );

        manager2
            .connect(
                node2.node_id().clone(),
                node1.node_id().clone(),
                node1.addresses().to_vec(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        let connections1 = manager.connections.lock().await;
        let connections2 = manager2.connections.lock().await;
        assert!(!connections1.is_empty());
        assert!(!connections2.is_empty());

        assert!(connections1.contains_key(&node2.node_id().clone()));
        assert!(connections2.contains_key(&node1.node_id().clone()));
    }

    #[tokio::test]
    async fn test_send_message() {
        init();
        let keypair1 = KeyPair::generate().expect("generate keypair");
        let keypair2 = KeyPair::generate().expect("generate keypair");

        let config = mock_quic_config();
        let manager = ConnectionManager::run_server(config).await;
        assert!(manager.is_ok());
        let manager = manager.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let addr1 = manager.endpoint.local_addr().expect("get local addr");
        let addr1 = format!("127.0.0.1:{}", addr1.port()).parse().unwrap();
        let node1 = Node::new(
            NodeId::from_keypair(&keypair1),
            "",
            vec![addr1],
            NodeType::Normal,
            keypair1.clone(),
        );

        let config2 = mock_quic_config2();
        let manager2 = ConnectionManager::run_server(config2).await;
        assert!(manager2.is_ok());
        let manager2 = manager2.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let addr2 = manager2.endpoint.local_addr().expect("get local addr");
        let node2 = Node::new(
            NodeId::from_keypair(&keypair2),
            "",
            vec![addr2],
            NodeType::Normal,
            keypair2.clone(),
        );

        manager2
            .connect(
                node2.node_id().clone(),
                node1.node_id().clone(),
                node1.addresses().to_vec(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        {
            let connections1 = manager.connections.lock().await;
            let connections2 = manager2.connections.lock().await;
            assert!(!connections1.is_empty());
            assert!(!connections2.is_empty());
        }

        manager2
            .send_message(node1.node_id().clone(), b"hello".to_vec())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
