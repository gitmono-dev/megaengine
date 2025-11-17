use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;

use megaengine::gossip::GossipService;
use megaengine::{
    git::{repo_name_space, repo_root_commit_bytes},
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("megaengine=info".parse().unwrap()),
        )
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
                    alias,
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
                    let gossip = std::sync::Arc::new(GossipService::new(
                        std::sync::Arc::clone(conn_mgr),
                        node.clone(),
                        None,
                    ));
                    tokio::spawn(gossip.start());
                    tracing::info!("Gossip protocol started");
                } else {
                    tracing::warn!("No connection manager found, gossip not started");
                }

                println!(
                    "Node started successfully: {} ({})",
                    node.node_id().0,
                    node.alias()
                );
                println!("Listening on: {}", addr);
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
        Commands::Repo { action } => {
            match action {
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
                            println!("Ensure the provided path is a git repository with at least one commit");
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

                    let repo = repo::repo::Repo::new(
                        repo_id.to_string(),
                        desc,
                        std::path::PathBuf::from(path),
                    );

                    let mut manager = repo::repo_manager::RepoManager::new();
                    match manager.register_repo(repo).await {
                        Ok(_) => tracing::info!("Repo {} added", repo_id),
                        Err(e) => tracing::info!("Failed to add repo: {}", e),
                    }
                }
            }
        }
    }

    Ok(())
}
