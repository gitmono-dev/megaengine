use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(None)")]
pub enum MessageStatus {
    #[sea_orm(string_value = "Sending")]
    Sending,
    #[sea_orm(string_value = "Sent")]
    Sent,
    #[sea_orm(string_value = "Delivered")]
    Delivered,
    #[sea_orm(string_value = "Failed")]
    Failed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "chat_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String, // UUID
    pub from: String,    // Sender NodeId
    pub to: String,      // Receiver NodeId
    pub content: String, // Plaintext content (local storage is trusted for now)
    pub created_at: i64, // Timestamp
    pub status: MessageStatus,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

use anyhow::Result;
use sea_orm::{ActiveModelTrait, Set};

pub async fn save_message(
    id: String,
    from: String,
    to: String,
    content: String,
    created_at: i64,
    status: MessageStatus,
) -> Result<()> {
    let db = crate::storage::get_db_conn().await?;
    let model = ActiveModel {
        id: Set(id),
        from: Set(from),
        to: Set(to),
        content: Set(content),
        created_at: Set(created_at),
        status: Set(status),
    };
    model.insert(&db).await?;
    Ok(())
}

pub async fn update_message_status(msg_id: &str, status: MessageStatus) -> Result<()> {
    let db = crate::storage::get_db_conn().await?;
    let msg = Entity::find_by_id(msg_id).one(&db).await?;
    if let Some(m) = msg {
        let mut active: ActiveModel = m.into();
        active.status = Set(status);
        active.update(&db).await?;
    }
    Ok(())
}
