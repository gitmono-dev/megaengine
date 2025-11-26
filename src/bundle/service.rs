use crate::bundle::transfer::BundleMessageType;
use crate::bundle::transfer::BundleTransferManager;
use crate::node::node_id::NodeId;
use crate::transport::quic::ConnectionManager;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// Bundle 传输服务
///
/// 负责处理 bundle 文件的接收和发送，
pub struct BundleService {
    connection_manager: Arc<Mutex<ConnectionManager>>,
    bundle_manager: Arc<BundleTransferManager>,
}
pub struct BundleRequest {
    pub requester_id: NodeId,
    pub repo_id: String,
}

impl BundleService {
    /// 创建新的 BundleService
    pub fn new(connection_manager: Arc<Mutex<ConnectionManager>>, storage_dir: PathBuf) -> Self {
        let bundle_manager = Arc::new(BundleTransferManager::new(
            connection_manager.clone(),
            storage_dir,
        ));

        Self {
            connection_manager,
            bundle_manager,
        }
    }

    /// 启动 Bundle 服务：注册 data_sender 并处理接收的 bundle 消息
    pub async fn start(self: Arc<Self>) -> Result<()> {
        // 注册数据传输接收器
        let (data_tx, mut data_rx) = mpsc::channel::<(NodeId, Vec<u8>)>(256);

        {
            let mgr = self.connection_manager.lock().await;
            mgr.register_data_sender(data_tx).await;
        }

        // Bundle 数据处理任务
        let s = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some((from, data)) = data_rx.recv().await {
                if let Err(e) = s.bundle_manager.handle_bundle_message(from, data).await {
                    tracing::warn!("Failed to handle bundle message: {}", e);
                }
            }
        });

        Ok(())
    }

    /// 发送 bundle 文件到指定节点
    pub async fn send_bundle(
        &self,
        target_node_id: NodeId,
        repo_id: String,
        bundle_path: &str,
    ) -> Result<()> {
        self.bundle_manager
            .send_bundle(target_node_id, repo_id, bundle_path)
            .await
    }

    /// 获取接收的 bundle 文件路径
    pub fn get_bundle_path(&self, from: &NodeId, repo_id: &str) -> PathBuf {
        self.bundle_manager.get_bundle_path(from, repo_id)
    }

    /// 向指定节点请求 bundle（发送 Request 消息）
    pub async fn request_bundle(&self, target_node_id: &NodeId, repo_id: &str) -> Result<()> {
        // 构造 Request 消息
        let start_msg = BundleMessageType::Request {
            repo_id: repo_id.to_string(),
        };

        let payload = serde_json::to_vec(&start_msg)?;

        let mgr = self.connection_manager.lock().await;
        let peers = mgr.list_peers().await;
        tracing::info!("Sent bundle peers: {:?}", peers);
        mgr.send_data_message(target_node_id.clone(), payload)
            .await?;

        tracing::info!(
            "Bundle request sent to {} for repo {}",
            target_node_id,
            repo_id
        );

        Ok(())
    }
}
