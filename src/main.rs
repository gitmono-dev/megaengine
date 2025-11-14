use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;

use megaengine::storage;

#[derive(Parser)]
#[command(name = "megaengine")]
#[command(about = "MegaEngine P2P Git", long_about = None)]
struct Cli {
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
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize rustls crypto provider
    let _ = rustls::crypto::ring::default_provider().install_default();

    // init logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Auth { action } => match action {
            AuthAction::Init => {
                tracing::info!("Generating new keypair...");
                let kp = megaengine::identity::keypair::KeyPair::generate()?;
                storage::save_keypair(&kp)?;
                tracing::info!("Keypair saved to {:?}", storage::keypair_path());
            }
        },
        Commands::Node { action } => match action {
            NodeAction::Start {
                alias,
                addr,
                cert_path,
            } => {
                tracing::info!("Starting node...");

                // Ensure certificates exist, generate if needed
                let cert_dir = &cert_path;
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

                // parse addresses
                let mut addrs: Vec<SocketAddr> = Vec::new();
                addrs.push(addr.parse()?);

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

                // Create QUIC config for this node
                let quic_config = megaengine::transport::config::QuicConfig::new(
                    addr.parse()?,
                    format!("{}/cert.pem", cert_dir),
                    format!("{}/key.pem", cert_dir),
                    format!("{}/ca-cert.pem", cert_dir),
                );

                // Start QUIC server and keep it running
                tracing::info!("Starting QUIC server on {}...", addr);
                node.start_quic_server(quic_config).await?;

                println!(
                    "Node started successfully: {} ({})",
                    node.node_id().0,
                    node.alias()
                );
                println!("Listening on: {}", addr);
                println!("Press Ctrl+C to stop");

                // Keep the node running indefinitely
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        },
    }

    Ok(())
}
