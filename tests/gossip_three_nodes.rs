//! 集成测试：启动三个节点，gossip 传递消息
use megaengine::gossip::{GossipService, SignedMessage};
use megaengine::identity::keypair::KeyPair;
use megaengine::node::node::{Node, NodeType};
use megaengine::storage::node_model;
use megaengine::transport::config::QuicConfig;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_gossip_three_nodes_message_relay() {
    // 初始化 rustls crypto provider
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 初始化日志
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();

    // 生成或确保证书存在
    megaengine::transport::cert::ensure_certificates(
        "cert/cert.pem",
        "cert/key.pem",
        "cert/ca-cert.pem",
    )
    .expect("ensure certificates");

    megaengine::transport::cert::ensure_certificates(
        "cert/cert2.pem",
        "cert/key2.pem",
        "cert/ca-cert.pem",
    )
    .expect("ensure certificates 2");

    megaengine::transport::cert::ensure_certificates(
        "cert/cert3.pem",
        "cert/key3.pem",
        "cert/ca-cert.pem",
    )
    .expect("ensure certificates 3");

    // 1. 生成三对密钥
    let kp1 = KeyPair::generate().unwrap();
    let kp2 = KeyPair::generate().unwrap();
    let kp3 = KeyPair::generate().unwrap();

    // 2. 分配三个端口
    let addr1: SocketAddr = "127.0.0.1:19001".parse().unwrap();
    let addr2: SocketAddr = "127.0.0.1:19002".parse().unwrap();
    let addr3: SocketAddr = "127.0.0.1:19003".parse().unwrap();

    // 3. 创建节点
    let mut node1 = Node::from_keypair(&kp1, "node1", vec![addr1], NodeType::Normal);
    let mut node2 = Node::from_keypair(&kp2, "node2", vec![addr2], NodeType::Normal);
    let mut node3 = Node::from_keypair(&kp3, "node3", vec![addr3], NodeType::Normal);

    // 4. 启动 QUIC server
    let config1 = QuicConfig::new(
        addr1,
        "cert/cert.pem".to_string(),
        "cert/key.pem".to_string(),
        "cert/ca-cert.pem".to_string(),
    );
    let config2 = QuicConfig::new(
        addr2,
        "cert/cert2.pem".to_string(),
        "cert/key2.pem".to_string(),
        "cert/ca-cert.pem".to_string(),
    );
    let config3 = QuicConfig::new(
        addr3,
        "cert/cert3.pem".to_string(),
        "cert/key3.pem".to_string(),
        "cert/ca-cert.pem".to_string(),
    );
    node1.start_quic_server(config1).await.unwrap();
    node2.start_quic_server(config2).await.unwrap();
    node3.start_quic_server(config3).await.unwrap();

    // 5. 启动 gossip 和 bundle 服务
    let bundle_storage = std::path::PathBuf::from("./data/test_bundles");
    let gossip1 = Arc::new(GossipService::new(
        Arc::clone(node1.connection_manager.as_ref().unwrap()),
        node1.clone(),
        None,
    ));
    let gossip2 = Arc::new(GossipService::new(
        Arc::clone(node2.connection_manager.as_ref().unwrap()),
        node2.clone(),
        None,
    ));
    let gossip3 = Arc::new(GossipService::new(
        Arc::clone(node3.connection_manager.as_ref().unwrap()),
        node3.clone(),
        None,
    ));
    gossip1.start().await.unwrap();
    gossip2.start().await.unwrap();
    gossip3.start().await.unwrap();

    // 6. 连接成链 node1 <-> node2 <-> node3
    let mgr1 = node1.connection_manager.as_ref().unwrap().clone();
    let mgr2 = node2.connection_manager.as_ref().unwrap().clone();
    let mgr3 = node3.connection_manager.as_ref().unwrap().clone();
    mgr1.lock()
        .await
        .connect(
            node1.node_id().clone(),
            node2.node_id().clone(),
            vec![addr2],
        )
        .await
        .unwrap();
    mgr2.lock()
        .await
        .connect(
            node2.node_id().clone(),
            node3.node_id().clone(),
            vec![addr3],
        )
        .await
        .unwrap();
    // 等待连接建立
    sleep(Duration::from_millis(500)).await;

    // 7. node1 发送 gossip 消息（NodeAnnouncement）
    let signed = SignedMessage::new_node_sign_message(node1.clone()).unwrap();
    let env = serde_json::to_vec(&serde_json::json!({"payload": signed, "ttl": 3})).unwrap();
    mgr1.lock()
        .await
        .send_message(node2.node_id().clone(), env)
        .await
        .unwrap();

    // 8. 等待消息传播
    sleep(Duration::from_secs(1)).await;

    // 8. node1 发送 gossip 消息（NodeAnnouncement）
    let signed = SignedMessage::new_node_sign_message(node3.clone()).unwrap();
    let env = serde_json::to_vec(&serde_json::json!({"payload": signed, "ttl": 3})).unwrap();
    mgr3.lock()
        .await
        .send_message(node2.node_id().clone(), env)
        .await
        .unwrap();

    // 这里只能通过日志人工观察传播效果，或后续扩展 GossipService 提供 hook/回调收集消息
    sleep(Duration::from_secs(1)).await;

    // Cleanup: Remove nodes from database
    let node_id_1 = node1.node_id().to_string();
    let node_id_2 = node2.node_id().to_string();
    let node_id_3 = node3.node_id().to_string();
    let _ = node_model::delete_node_from_db(&node_id_1).await;
    let _ = node_model::delete_node_from_db(&node_id_2).await;
    let _ = node_model::delete_node_from_db(&node_id_3).await;
}
