use anyhow::Result;
use clap::Subcommand;
use megaengine::node::node_id::NodeId;
use megaengine::storage::chat_message::{Entity as ChatMessage, MessageStatus};
use megaengine::util::timestamp_now;
use sea_orm::{EntityTrait, QueryOrder};
use uuid::Uuid;

#[derive(Clone, Debug, Subcommand)]
pub enum ChatCommand {
    /// Send a message to a node
    Send {
        /// Target Node ID (did:key:...)
        #[arg(long)]
        to: String,
        /// Message content
        #[arg(long)]
        msg: String,
    },
    /// List messages
    List,
}

pub async fn run_chat_command(cmd: ChatCommand) -> Result<()> {
    match cmd {
        ChatCommand::Send { to, msg } => {
            // Load identity
            let keypair = megaengine::storage::load_keypair()?;
            let my_node_id = NodeId::from_keypair(&keypair);

            let msg_id = Uuid::new_v4().to_string();

            // Save to DB (Queue it)
            // The background node process (if running) will pick this up and send it.
            // If it's not running, it will be sent next time it runs.
            megaengine::storage::chat_message::save_message(
                msg_id.clone(),
                my_node_id.to_string(),
                to.clone(),
                msg,
                timestamp_now(),
                MessageStatus::Sending,
            )
            .await?;

            println!("Message queued (ID: {}).", msg_id);
            println!("It will be delivered automatically when the node service is active.");
        }
        ChatCommand::List => {
            let db = megaengine::storage::get_db_conn().await?;
            let messages = ChatMessage::find()
                .order_by_desc(megaengine::storage::chat_message::Column::CreatedAt)
                .all(&db)
                .await?;

            println!("--- Chat History ---");
            for m in messages {
                let time = if let Some(dt) = chrono::DateTime::from_timestamp(m.created_at, 0) {
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                } else {
                    m.created_at.to_string()
                };
                println!(
                    "[{}] From: {} To: {} : {} ({:?})",
                    time, m.from, m.to, m.content, m.status
                );
            }
        }
    }
    Ok(())
}
