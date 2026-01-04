use anyhow::Result;
use megaengine::{
    bundle::BundleService, node::node_addr::NodeAddr, storage, transport::config::QuicConfig,
};
use std::path::PathBuf;
use std::sync::Arc;

pub async fn handle_node_start(
    root_path: &str,
    alias: String,
    addr: String,
    cert_path: String,
    bootstrap_node: Option<String>,
) -> Result<()> {
    tracing::info!("Starting node...");
    let cert_dir = format!("{}/{}", root_path, cert_path);
    megaengine::transport::cert::ensure_certificates(
        &format!("{}/cert.pem", cert_dir),
        &format!("{}/key.pem", cert_dir),
        &format!("{}/ca-cert.pem", cert_dir),
    )?;

    let kp = match storage::load_keypair() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("failed to load keypair: {}", e);
            tracing::info!("Run `auth init` first to generate keys");
            return Ok(());
        }
    };

    let addrs: Vec<std::net::SocketAddr> = vec![addr.parse()?];

    let mut node = megaengine::node::node::Node::from_keypair(
        &kp,
        &alias,
        addrs.clone(),
        megaengine::node::node::NodeType::Normal,
    );
    tracing::info!(
        "Node initialized: alias={} id={}",
        node.alias(),
        node.node_id().0
    );

    let quic_config = QuicConfig::new(
        addr.parse()?,
        format!("{}/cert.pem", cert_dir),
        format!("{}/key.pem", cert_dir),
        format!("{}/ca-cert.pem", cert_dir),
    );

    tracing::info!("Starting QUIC server on {}...", addr);
    node.start_quic_server(quic_config).await?;

    if let Some(conn_mgr) = &node.connection_manager {
        // 启动 Gossip 服务
        let gossip = Arc::new(megaengine::gossip::GossipService::new(
            Arc::clone(conn_mgr),
            node.clone(),
            None,
        ));
        tokio::spawn(gossip.start());
        tracing::info!("Gossip protocol started");

        // 启动 Bundle 传输服务
        let bundles_dir = PathBuf::from(format!("{}/bundles", root_path));
        let bundle_storage = bundles_dir.clone();
        let bundle_service = Arc::new(BundleService::new(Arc::clone(conn_mgr), bundle_storage));
        tokio::spawn(bundle_service.clone().start());
        tracing::info!("Bundle transfer service started");

        // 启动 Bundle 同步后台任务
        let bundle_service_for_sync = Arc::new(tokio::sync::Mutex::new(BundleService::new(
            Arc::clone(conn_mgr),
            bundles_dir,
        )));
        megaengine::bundle::start_bundle_sync_task(bundle_service_for_sync).await;
        tracing::info!("Bundle sync task started");

        // 启动 Repo 同步后台任务
        megaengine::repo::start_repo_sync_task().await;
        tracing::info!("Repo sync task started");
    } else {
        tracing::warn!("No connection manager found, services not started");
    }

    // 连接到 bootstrap node
    if let Some(bootstrap_addr_str) = bootstrap_node {
        connect_to_bootstrap_node(&node, bootstrap_addr_str).await;
    }

    println!(
        "Node started successfully: {} ({})",
        node.node_id().0,
        node.alias()
    );
    println!("Listening on: {}", addr);

    let node_addr = NodeAddr::new(node.node_id().clone(), addr.parse()?);
    println!("Node address: {}", node_addr);
    println!("Press Ctrl+C to stop");

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

async fn connect_to_bootstrap_node(
    node: &megaengine::node::node::Node,
    bootstrap_addr_str: String,
) {
    if let Some(conn_mgr) = &node.connection_manager {
        tracing::info!(
            "Attempting to connect to bootstrap node: {}",
            bootstrap_addr_str
        );

        match NodeAddr::parse(&bootstrap_addr_str) {
            Ok(bootstrap_info) => {
                match conn_mgr
                    .lock()
                    .await
                    .connect(
                        node.node_id().clone(),
                        bootstrap_info.peer_id.clone(),
                        vec![bootstrap_info.address],
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Successfully connected to bootstrap node {} at {}",
                            bootstrap_info.peer_id,
                            bootstrap_info.address
                        );
                        println!(
                            "Connected to bootstrap node: {} at {}",
                            bootstrap_info.peer_id, bootstrap_info.address
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to connect to bootstrap node: {}", e);
                        eprintln!("Warning: Failed to connect to bootstrap node: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to parse bootstrap node address: {}", e);
                eprintln!("Error: {}", e);
            }
        }
    }
}

pub async fn handle_node_id() -> Result<()> {
    let kp = match storage::load_keypair() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("failed to load keypair: {}", e);
            tracing::info!("Run `auth init` first to generate keys");
            return Ok(());
        }
    };

    let node_id = megaengine::node::node_id::NodeId::from_keypair(&kp);
    println!("{}", node_id);
    Ok(())
}

pub async fn handle_node(root_path: String, action: crate::NodeAction) -> Result<()> {
    match action {
        crate::NodeAction::Start {
            alias,
            addr,
            cert_path,
            bootstrap_node,
        } => handle_node_start(&root_path, alias, addr, cert_path, bootstrap_node).await,
        crate::NodeAction::Id => handle_node_id().await,
    }
}
