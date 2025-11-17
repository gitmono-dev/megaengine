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

use tokio::sync::mpsc::Sender as TokioSender;

const READ_BUF_SIZE: usize = 1024 * 1024;

// Type alias for incoming message sender to reduce type complexity
type IncomingMessageSender = Arc<Mutex<Option<TokioSender<(NodeId, Vec<u8>)>>>>;

#[derive(Debug, Clone)]
pub struct ConnectionManager {
    #[allow(dead_code)]
    config: QuicConfig,
    endpoint: Arc<Endpoint>,
    connection_tx: mpsc::Sender<QuicConnection>,
    connections: Arc<Mutex<HashMap<NodeId, Arc<QuicConnection>>>>,
    incoming_sender: IncomingMessageSender,
}

#[derive(Debug, Clone)]
pub struct QuicConnection {
    pub connection: Connection,
    pub peer_addr: SocketAddr,
    pub node_id: NodeId,
    pub connection_type: ConnectionType,
    pub connection_state: ConnectionState,
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Failed,
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
            incoming_sender: Arc::new(Mutex::new(None)),
        };
        Ok((transport, connection_rx))
    }

    pub async fn run_server(config: QuicConfig) -> Result<Self> {
        let (manager, mut conn_rx) = ConnectionManager::server(config)?;
        let endpoint = Arc::clone(&manager.endpoint);
        let connection_tx = manager.connection_tx.clone();
        let connections = Arc::clone(&manager.connections);
        let manager_clone = manager.clone();

        tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                info!("Accepting connection from {}", incoming.remote_address());
                let tx = connection_tx.clone();
                let manager_clone = manager_clone.clone();
                tokio::spawn(async move {
                    // let manager_clone = manager_clone.clone();
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
                connection_state: ConnectionState::Connected,
            },
            message_rx,
        ))
    }

    /// 生成消息处理任务，将接收到的消息转发到注册的 incoming_sender（包含发送者 NodeId）
    async fn spawn_message_handler(&self, peer_id: NodeId, mut receiver: Receiver<Vec<u8>>) {
        let incoming = Arc::clone(&self.incoming_sender);
        tokio::spawn(async move {
            while let Some(data) = receiver.recv().await {
                // forward to registered gossip handler if present
                let maybe = incoming.lock().await;
                if let Some(tx) = maybe.as_ref() {
                    let _ = tx.send((peer_id.clone(), data)).await;
                } else {
                    let message = String::from_utf8(data).unwrap_or_default();
                    info!("Received message from {}: {}", peer_id, message);
                }
            }
        });
    }

    /// Register a channel to receive incoming messages from peers: (peer_id, bytes)
    pub async fn register_incoming_sender(&self, tx: TokioSender<(NodeId, Vec<u8>)>) {
        let mut guard = self.incoming_sender.lock().await;
        *guard = Some(tx);
    }

    /// Return list of connected peer NodeIds
    pub async fn list_peers(&self) -> Vec<NodeId> {
        let connections = self.connections.lock().await;
        connections.keys().cloned().collect()
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
            connection,
            peer_addr,
            node_id: target_node_id.clone(),
            connection_type: ConnectionType::Client,
            connection_state: ConnectionState::Connected,
        };
        let connections = Arc::clone(&self.connections);
        connections
            .lock()
            .await
            .insert(target_node_id.clone(), Arc::from(quic_conn.clone()));

        Ok(())
    }

    pub async fn send_message(&self, node_id: NodeId, message: Vec<u8>) -> Result<()> {
        let connections = self.connections.lock().await;
        let conn = connections.get(&node_id).context(format!(
            "Failed to send message to node[{}], connection not found",
            node_id
        ))?;
        let mut sender = conn.connection.open_uni().await?;
        sender.write_all(message.as_slice()).await?;
        sender.finish()?;
        Ok(())
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
