use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
use cli::{handle_auth, handle_node, handle_repo};

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
    /// Update repository from bundle (like git pull)
    Pull {
        /// Repository ID
        #[arg(long)]
        repo_id: String,
    },
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
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error,megaengine=debug"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_level(true)
        .init();

    let cli = Cli::parse();

    let root_path = resolve_root_path(&cli.root)?;

    match cli.command {
        Commands::Auth { action } => match action {
            AuthAction::Init => {
                handle_auth().await?;
            }
        },
        Commands::Node { action } => {
            handle_node(root_path, String::new(), String::new(), String::new(), None, action)
                .await?;
        }
        Commands::Repo { action } => {
            handle_repo(action).await?;
        }
    }

    Ok(())
}

fn resolve_root_path(root_arg: &str) -> Result<String> {
    if let Ok(env_root) = std::env::var("MEGAENGINE_ROOT") {
        return Ok(env_root);
    }

    let path = if root_arg.starts_with("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        root_arg.replace("~", &home)
    } else {
        root_arg.to_string()
    };

    std::env::set_var("MEGAENGINE_ROOT", &path);
    Ok(path)
}
