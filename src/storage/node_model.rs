use std::net::SocketAddr;

use anyhow::Result;
use sea_orm::entity::prelude::*;
use sea_orm::Set;

use crate::node::node::{NodeInfo, NodeType};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "nodes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub alias: String,
    pub addresses: String,
    pub node_type: i32,
    pub version: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// 将 NodeInfo 保存到数据库
pub async fn save_node_info_to_db(info: &NodeInfo) -> Result<()> {
    let db = crate::storage::get_db_conn().await?;

    let addresses_json = serde_json::to_string(&info.addresses)?;
    let now = chrono::Local::now().timestamp();

    // 删除旧记录（如果存在）
    let _ = Entity::delete_by_id(info.node_id.to_string())
        .exec(&db)
        .await;

    let node_type_int = match info.node_type {
        NodeType::Normal => 0,
        NodeType::Relay => 1,
    };

    let active = ActiveModel {
        id: Set(info.node_id.to_string()),
        alias: Set(info.alias.clone()),
        addresses: Set(addresses_json),
        node_type: Set(node_type_int),
        version: Set(info.version as i32),
        created_at: Set(now),
        updated_at: Set(now),
    };

    Entity::insert(active).exec(&db).await?;
    Ok(())
}

/// 从数据库加载 NodeInfo
pub async fn load_node_info_from_db(node_id: &str) -> Result<Option<NodeInfo>> {
    let db = crate::storage::get_db_conn().await?;

    if let Some(m) = Entity::find_by_id(node_id).one(&db).await? {
        let addresses: Vec<SocketAddr> = serde_json::from_str(&m.addresses)?;
        let node_type = match m.node_type {
            0 => NodeType::Normal,
            _ => NodeType::Relay,
        };

        let info = NodeInfo {
            node_id: crate::node::node_id::NodeId::from_string(&m.id)
                .unwrap_or_else(|_| crate::node::node_id::NodeId::from_string("").unwrap()),
            alias: m.alias,
            addresses,
            node_type,
            version: m.version as u8,
        };
        Ok(Some(info))
    } else {
        Ok(None)
    }
}

/// 删除节点记录
pub async fn delete_node_from_db(node_id: &str) -> Result<()> {
    let db = crate::storage::get_db_conn().await?;
    Entity::delete_by_id(node_id).exec(&db).await?;
    Ok(())
}

/// 列出所有节点
pub async fn list_nodes() -> Result<Vec<NodeInfo>> {
    let db = crate::storage::get_db_conn().await?;
    let models = Entity::find().all(&db).await?;

    let mut out = Vec::new();
    for m in models {
        let addresses: Vec<SocketAddr> = serde_json::from_str(&m.addresses).unwrap_or_default();
        let node_type = match m.node_type {
            0 => NodeType::Normal,
            _ => NodeType::Relay,
        };
        let info = NodeInfo {
            node_id: crate::node::node_id::NodeId::from_string(&m.id)
                .unwrap_or_else(|_| crate::node::node_id::NodeId::from_string("").unwrap()),
            alias: m.alias,
            addresses,
            node_type,
            version: m.version as u8,
        };
        out.push(info);
    }
    Ok(out)
}
