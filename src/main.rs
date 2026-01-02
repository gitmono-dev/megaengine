use anyhow::Result;
use clap::{Parser, Subcommand};
use megaengine::bundle::BundleService;
use megaengine::git::git_repo::{repo_name_space, repo_root_commit_bytes};
use megaengine::git::pack::restore_repo_from_bundle;
use megaengine::node::node_addr::NodeAddr;
use std::net::SocketAddr;
use std::path::PathBuf;

use megaengine::gossip::GossipService;
use megaengine::{
    node::node_id::NodeId,
    repo::{self, repo_id::RepoId},
    storage,
    util::timestamp_now,
};

#[derive(Parser)]
#[command(name = "megaengine")]
#[command(about = "MegaEngine P2P Git", long_about = None)]
struct Cli {
    /// Root data directory (overrides $MEGAENGINE_ROOT). Defaults to ~/.megaengine
    #[arg(long, global = true, default_value = "~/.megaengine")]
    root: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Identity related commands
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Node related commands
    Node {
        #[command(subcommand)]
        action: NodeAction,
    },
    /// Repo related commands
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Generate and save a new keypair
    Init,
}

#[derive(Subcommand)]
enum NodeAction {
    /// Start node (initialization)
    Start {
        /// node alias
        #[arg(long, default_value = "mega-node")]
        alias: String,
        /// one or more listen/announce addresses, e.g. 0.0.0.0:9000
        #[arg(short, long, default_value = "0.0.0.0:9000")]
        addr: String,

        #[arg(short, long, default_value = "cert")]
        cert_path: String,

        /// Bootstrap node address to connect to on startup (e.g., 127.0.0.1:9000)
        #[arg(long)]
        bootstrap_node: Option<String>,
    },
    /// Print node id using stored keypair
    Id,
}

#[derive(Subcommand)]
enum RepoAction {
    /// Add a repository record to the manager and database
    Add {
        /// Local path to the repository
        #[arg(long)]
        path: String,

        /// Description
        #[arg(long, default_value = "")]
        description: String,
    },
    /// List all repositories
    List,
    Clone {
        #[arg(long)]
        output: String,

        #[arg(long)]
        repo_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 初始化 tracing 日志
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error,megaengine=info"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_level(true)
        .init();

    let cli = Cli::parse();

    let root_path = if let Ok(env_root) = std::env::var("MEGAENGINE_ROOT") {
        env_root
    } else {
        let path = if cli.root.starts_with("~/") {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            cli.root.replace("~", &home)
        } else {
            cli.root.clone()
        };
        std::env::set_var("MEGAENGINE_ROOT", &path);
        path
    };

    match cli.command {
        Commands::Auth { action } => match action {
            AuthAction::Init => {
                let kp_path = storage::keypair_path();
                if kp_path.exists() {
                    tracing::info!(
                        "Keypair already exists at {:?}; skipping generation",
                        kp_path
                    );
                } else {
                    tracing::info!("Generating new keypair...");
                    let kp = megaengine::identity::keypair::KeyPair::generate()?;
                    storage::save_keypair(&kp)?;
                    tracing::info!("Keypair saved to {:?}", storage::keypair_path());
                }
            }
        },
        Commands::Node { action } => match action {
            NodeAction::Start {
                alias,
                addr,
                cert_path,
                bootstrap_node,
            } => {
                tracing::info!("Starting node...");
                let cert_dir = format!("{}/{}", &root_path, cert_path);
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

                let addrs: Vec<SocketAddr> = vec![addr.parse()?];

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

                let quic_config = megaengine::transport::config::QuicConfig::new(
                    addr.parse()?,
                    format!("{}/cert.pem", cert_dir),
                    format!("{}/key.pem", cert_dir),
                    format!("{}/ca-cert.pem", cert_dir),
                );

                tracing::info!("Starting QUIC server on {}...", addr);
                node.start_quic_server(quic_config).await?;

                if let Some(conn_mgr) = &node.connection_manager {
                    // 启动 Gossip 服务
                    let gossip = std::sync::Arc::new(GossipService::new(
                        std::sync::Arc::clone(conn_mgr),
                        node.clone(),
                        None,
                    ));
                    tokio::spawn(gossip.start());
                    tracing::info!("Gossip protocol started");

                    // 启动 Bundle 传输服务
                    let bundles_dir = PathBuf::from(format!("{}/bundles", &root_path));
                    let bundle_storage = bundles_dir.clone();
                    let bundle_service = std::sync::Arc::new(BundleService::new(
                        std::sync::Arc::clone(conn_mgr),
                        bundle_storage,
                    ));
                    tokio::spawn(bundle_service.clone().start());
                    tracing::info!("Bundle transfer service started");

                    // 启动 Bundle 同步后台任务：定时检查 external repos 并请求 bundle
                    let bundle_service_for_sync = std::sync::Arc::new(tokio::sync::Mutex::new(
                        BundleService::new(std::sync::Arc::clone(conn_mgr), bundles_dir),
                    ));
                    megaengine::bundle::start_bundle_sync_task(bundle_service_for_sync).await;
                } else {
                    tracing::warn!("No connection manager found, services not started");
                }

                // 如果提供了 bootstrap_node，尝试连接到它
                if let Some(bootstrap_addr_str) = bootstrap_node {
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
                                        tracing::warn!(
                                            "Failed to connect to bootstrap node: {}",
                                            e
                                        );
                                        eprintln!(
                                            "Warning: Failed to connect to bootstrap node: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to parse bootstrap node address: {}", e);
                                eprintln!("Error: {}", e);
                                return Err(e);
                            }
                        }
                    }
                }

                println!(
                    "Node started successfully: {} ({})",
                    node.node_id().0,
                    node.alias()
                );
                println!("Listening on: {}", addr);

                // 打印 node 地址
                let addr = NodeAddr::new(node.node_id().clone(), addr.parse()?);
                println!("Node address: {}", addr);

                println!("Press Ctrl+C to stop");

                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
            NodeAction::Id => {
                let kp = match storage::load_keypair() {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::error!("failed to load keypair: {}", e);
                        tracing::info!("Run `auth init` first to generate keys");
                        return Ok(());
                    }
                };

                let node_id = NodeId::from_keypair(&kp);
                println!("{}", node_id);
            }
        },
        Commands::Repo { action } => match action {
            RepoAction::Add { path, description } => {
                let kp = match storage::load_keypair() {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::error!("failed to load keypair: {}", e);
                        tracing::info!("Run `auth init` first to generate keys");
                        return Ok(());
                    }
                };
                let node_id = NodeId::from_keypair(&kp);

                let root_bytes = match repo_root_commit_bytes(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!("failed to read repo root commit: {}", e);
                        println!(
                            "Ensure the provided path is a git repository with at least one commit"
                        );
                        return Ok(());
                    }
                };

                let repo_id =
                    match RepoId::generate(root_bytes.as_slice(), &kp.verifying_key_bytes()) {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::error!("Failed to generate RepoId: {}", e);
                            return Ok(());
                        }
                    };

                let name = repo_name_space(&path);
                let desc = repo::repo::P2PDescription {
                    creator: node_id.to_string(),
                    name: name.clone(),
                    description: description.clone(),
                    timestamp: timestamp_now(),
                };

                let mut repo_obj = repo::repo::Repo::new(repo_id.to_string(), desc, PathBuf::from(path.clone()));

                // Read and populate refs from the git repository
                match megaengine::git::git_repo::read_repo_refs(&path) {
                    Ok(refs) => {
                        repo_obj.refs = refs;
                        tracing::info!("Loaded {} refs from repository", repo_obj.refs.len());
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read refs from repository: {}", e);
                    }
                }

                let mut manager = repo::repo_manager::RepoManager::new();
                match manager.register_repo(repo_obj).await {
                    Ok(_) => tracing::info!("Repo {} added", repo_id),
                    Err(e) => tracing::info!("Failed to add repo: {}", e),
                }
            }
            RepoAction::List => match storage::repo_model::list_repos().await {
                Ok(repos) => {
                    if repos.is_empty() {
                        println!("No repositories found");
                    } else {
                        println!("Repositories:");
                        println!("{}", "─".repeat(100));
                        for repo in repos {
                            println!("  ID:          {}", repo.repo_id);
                            println!("  Name:        {}", repo.p2p_description.name);
                            println!("  Creator:     {}", repo.p2p_description.creator);
                            println!("  Description: {}", repo.p2p_description.description);
                            println!("  Path:        {}", repo.path.display());
                            println!("  Timestamp:   {}", repo.p2p_description.timestamp);
                            println!("{}", "─".repeat(100));
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to list repos: {}", e);
                    println!("Failed to list repositories: {}", e);
                }
            },
            RepoAction::Clone { output, repo_id } => {
                // 查询数据库获取 repo 信息
                match storage::repo_model::load_repo_from_db(&repo_id).await {
                    Ok(Some(repo)) => {
                        // 检查 bundle 是否存在
                        if repo.bundle.as_os_str().is_empty()
                            || repo.bundle.to_string_lossy().is_empty()
                        {
                            tracing::error!(
                                "Repository {} has no bundle available for cloning",
                                repo_id
                            );
                            println!("Error: Repository {} has no bundle available", repo_id);
                            return Ok(());
                        }

                        let bundle_path = repo.bundle.to_string_lossy().to_string();
                        if !std::path::Path::new(&bundle_path).exists() {
                            tracing::error!("Bundle file not found at path: {}", bundle_path);
                            println!("Error: Bundle file not found at {}", bundle_path);
                            return Ok(());
                        }

                        // 使用 git pack 模块中的函数恢复仓库
                        tracing::info!(
                            "Cloning repository {} from bundle {} to {}",
                            repo_id,
                            bundle_path,
                            output
                        );

                        match restore_repo_from_bundle(&bundle_path, &output).await {
                            Ok(_) => {
                                tracing::info!(
                                    "Repository {} cloned successfully to {}",
                                    repo_id,
                                    output
                                );
                                println!("✅ Repository cloned successfully to {}", output);
                                println!("  Repository: {}", repo.p2p_description.name);
                                println!("  Creator: {}", repo.p2p_description.creator);
                                println!("  Description: {}", repo.p2p_description.description);

                                // Read and save refs from the cloned repository
                                match megaengine::git::git_repo::read_repo_refs(&output) {
                                    Ok(refs) => {
                                        tracing::info!("Loaded {} refs from cloned repository", refs.len());
                                        // Save refs to the database
                                        match storage::ref_model::batch_save_refs(&repo_id, &refs).await {
                                            Ok(_) => {
                                                tracing::info!("Refs saved to database for repository {}", repo_id);
                                                println!("  Refs: {} branches/tags", refs.len());
                                            }
                                            Err(e) => {
                                                tracing::warn!("Failed to save refs to database: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to read refs from cloned repository: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to clone repository: {}", e);
                                println!("Error: Failed to clone repository: {}", e);
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::error!("Repository {} not found in database", repo_id);
                        println!("Error: Repository {} not found", repo_id);
                    }
                    Err(e) => {
                        tracing::error!("Failed to query repository {}: {}", repo_id, e);
                        println!("Error: Failed to query repository: {}", e);
                    }
                }
            }
        },
    }

    Ok(())
}
