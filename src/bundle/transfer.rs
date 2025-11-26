use crate::node::node_id::NodeId;
use crate::storage::repo_model;
use crate::transport::quic::ConnectionManager;
use crate::util::get_node_id_last_part;
use crate::util::get_repo_id_last_part;
use anyhow::Context;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Bundle 消息类型（用于多帧传输）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BundleMessageType {
    Request {
        repo_id: String,
    },
    /// 开始传输：包含文件元数据
    Start {
        repo_id: String,
        file_name: String,
        total_size: u64,
    },
    /// 数据块：包含分块数据
    Chunk {
        repo_id: String,
        chunk_idx: u32,
        data: Vec<u8>,
    },
    /// 传输完成
    Done {
        repo_id: String,
    },
}

/// Bundle 文件传输管理器
pub struct BundleTransferManager {
    connection_manager: Arc<Mutex<ConnectionManager>>,
    storage_dir: PathBuf,
}

impl BundleTransferManager {
    /// 创建新的 BundleTransferManager
    pub fn new(connection_manager: Arc<Mutex<ConnectionManager>>, storage_dir: PathBuf) -> Self {
        Self {
            connection_manager,
            storage_dir,
        }
    }

    /// 发送 bundle 文件到指定节点
    ///
    /// # Arguments
    /// * `target_node_id` - 目标节点 ID
    /// * `repo_id` - 仓库 ID
    /// * `bundle_path` - bundle 文件路径
    ///
    /// # Example
    /// ```ignore
    /// manager.send_bundle(peer_id, "repo123", "path/to/bundle").await?;
    /// ```
    pub async fn send_bundle(
        &self,
        target_node_id: NodeId,
        repo_id: String,
        bundle_path: &str,
    ) -> Result<()> {
        // 读取 bundle 文件
        let path = Path::new(bundle_path);
        let bundle_data = fs::read(path).await.context("Failed to read bundle file")?;

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo.bundle")
            .to_string();

        let total_size = bundle_data.len() as u64;

        info!(
            "Sending bundle {} ({} bytes) to node {}",
            file_name, total_size, target_node_id
        );

        let mgr = self.connection_manager.lock().await;

        // 1. 发送 START 消息
        let start_msg = BundleMessageType::Start {
            repo_id: repo_id.clone(),
            file_name: file_name.clone(),
            total_size,
        };
        let start_payload = serde_json::to_vec(&start_msg).context("Failed to serialize START")?;
        mgr.send_data_message(target_node_id.clone(), start_payload)
            .await
            .context("Failed to send START message")?;

        // 2. 分块发送数据
        const CHUNK_SIZE: usize = 64 * 1024; // 64KB per chunk
        for (chunk_idx, chunk) in bundle_data.chunks(CHUNK_SIZE).enumerate() {
            let chunk_msg = BundleMessageType::Chunk {
                repo_id: repo_id.clone(),
                chunk_idx: chunk_idx as u32,
                data: chunk.to_vec(),
            };
            let chunk_payload =
                serde_json::to_vec(&chunk_msg).context("Failed to serialize CHUNK")?;

            mgr.send_data_message(target_node_id.clone(), chunk_payload)
                .await
                .context("Failed to send CHUNK message")?;

            debug!(
                "Sent chunk {} ({} bytes) for repo {}",
                chunk_idx,
                chunk.len(),
                repo_id
            );
        }

        // 3. 发送 DONE 消息
        let done_msg = BundleMessageType::Done {
            repo_id: repo_id.clone(),
        };
        let done_payload = serde_json::to_vec(&done_msg).context("Failed to serialize DONE")?;
        mgr.send_data_message(target_node_id.clone(), done_payload)
            .await
            .context("Failed to send DONE message")?;

        info!(
            "Bundle {} sent successfully to node {} ({} chunks)",
            file_name,
            target_node_id,
            bundle_data.chunks(CHUNK_SIZE).count()
        );

        Ok(())
    }

    /// 处理接收的 bundle 消息流
    ///
    /// 这个方法应该由接收 data_sender 的处理器调用
    pub async fn handle_bundle_message(&self, from: NodeId, data: Vec<u8>) -> Result<()> {
        // 反序列化消息
        let msg: BundleMessageType =
            serde_json::from_slice(&data).context("Failed to deserialize bundle message")?;
        match msg {
            BundleMessageType::Request { repo_id } => {
                self.handle_bundle_request(&from, &repo_id).await
            }
            BundleMessageType::Start {
                repo_id,
                file_name,
                total_size,
            } => {
                self.handle_bundle_start(&from, &repo_id, &file_name, total_size)
                    .await
            }
            BundleMessageType::Chunk {
                repo_id,
                chunk_idx,
                data,
            } => {
                self.handle_bundle_chunk(&from, &repo_id, chunk_idx, data)
                    .await
            }
            BundleMessageType::Done { repo_id } => self.handle_bundle_done(&from, &repo_id).await,
        }
    }

    /// 将 NodeId 编码为合法的目录名（替换非法字符）
    fn encode_node_id(node_id: &NodeId) -> String {
        let id_str = node_id.to_string();
        get_node_id_last_part(&id_str)
    }

    /// 处理 Request 消息：检查本地 repo 是否存在，如果存在则生成 bundle 并发送
    async fn handle_bundle_request(&self, from: &NodeId, repo_id: &str) -> Result<()> {
        info!("Received bundle request from {} for repo {}", from, repo_id);

        // 检查本地是否有该 repo
        match crate::storage::repo_model::load_repo_from_db(repo_id).await {
            Ok(Some(repo)) => {
                // repo 存在，检查是否是本地 repo（不是 external）
                if repo.is_external {
                    warn!(
                        "Cannot send bundle for external repo {} to {}",
                        repo_id, from
                    );
                    return Ok(());
                }

                let repo_path = repo.path.to_string_lossy().to_string();
                let bundle_dir = self.storage_dir.clone();
                let bundle_file_name = format!("{}.bundle", get_repo_id_last_part(repo_id));
                let bundle_path = bundle_dir.join(&bundle_file_name);

                info!(
                    "Found local repo {} at {}, generating bundle for request from {}",
                    repo_id, repo_path, from
                );

                // 生成 bundle 文件（同步操作，需要在线程中运行）
                let repo_path_clone = repo_path.clone();
                let bundle_path_clone = bundle_path.clone();

                tokio::task::spawn_blocking(move || {
                    crate::git::pack::pack_repo_bundle(
                        &repo_path_clone,
                        bundle_path_clone.to_str().unwrap_or(""),
                    )
                })
                .await
                .context("Failed to spawn bundle packing task")??;

                info!("Bundle generated successfully for repo {}", repo_id);

                // 发送 bundle 给请求者
                self.send_bundle(
                    from.clone(),
                    repo_id.to_string(),
                    bundle_path.to_str().unwrap_or(""),
                )
                .await
                .context("Failed to send bundle in response to request")?;

                info!("Bundle for repo {} sent successfully to {}", repo_id, from);

                Ok(())
            }
            Ok(None) => {
                warn!(
                    "Received bundle request for non-existent repo {} from {}",
                    repo_id, from
                );
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Error checking repo {} for bundle request from {}: {}",
                    repo_id, from, e
                );
                Ok(())
            }
        }
    }

    /// 处理 START 消息
    async fn handle_bundle_start(
        &self,
        from: &NodeId,
        repo_id: &str,
        file_name: &str,
        total_size: u64,
    ) -> Result<()> {
        let encoded_id = Self::encode_node_id(from);
        let dir = self.storage_dir.join(&encoded_id);
        fs::create_dir_all(&dir)
            .await
            .context("Failed to create bundle storage directory")?;

        info!(
            "Bundle transfer START from {}: repo={}, file={}, size={} bytes",
            from, repo_id, file_name, total_size
        );

        Ok(())
    }

    /// 处理 CHUNK 消息
    async fn handle_bundle_chunk(
        &self,
        from: &NodeId,
        repo_id: &str,
        chunk_idx: u32,
        data: Vec<u8>,
    ) -> Result<()> {
        let encoded_id = Self::encode_node_id(from);
        let dir = self.storage_dir.join(&encoded_id);
        let encoded_repo_id = get_repo_id_last_part(repo_id);
        let file_path = dir.join(format!("{}.bundle", encoded_repo_id));

        // 追加写入到文件
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .context("Failed to open bundle file for appending")?;

        use tokio::io::AsyncWriteExt;
        file.write_all(&data)
            .await
            .context("Failed to write chunk data")?;

        info!(
            "Received chunk {} ({} bytes) for repo {} from {}",
            chunk_idx,
            data.len(),
            repo_id,
            from
        );

        Ok(())
    }

    /// 处理 DONE 消息
    async fn handle_bundle_done(&self, from: &NodeId, repo_id: &str) -> Result<()> {
        let encoded_id = Self::encode_node_id(from);
        let dir = self.storage_dir.join(&encoded_id);
        let encoded_repo_id = get_repo_id_last_part(repo_id);
        let file_path = dir.join(format!("{}.bundle", encoded_repo_id));

        if file_path.exists() {
            let metadata = fs::metadata(&file_path)
                .await
                .context("Failed to get bundle file metadata")?;
            // 标记 bundle 已接收
            let bundle_path = file_path.to_string_lossy().to_string();
            repo_model::update_repo_bundle(repo_id, &bundle_path).await?;
            info!(
                "Bundle transfer completed from {}: repo={}, file_size={} bytes",
                from,
                repo_id,
                metadata.len()
            );
        } else {
            warn!(
                "Bundle transfer DONE message received but file not found for repo {} from {}",
                repo_id, from
            );
        }

        Ok(())
    }

    /// 获取从指定节点接收的 bundle 文件路径
    pub fn get_bundle_path(&self, from: &NodeId, repo_id: &str) -> PathBuf {
        let encoded_id = Self::encode_node_id(from);
        self.storage_dir
            .join(&encoded_id)
            .join(format!("{}.bundle", repo_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_message_serialization() {
        let msg = BundleMessageType::Start {
            repo_id: "repo123".to_string(),
            file_name: "repo.bundle".to_string(),
            total_size: 1024,
        };

        let serialized = serde_json::to_vec(&msg).unwrap();
        let deserialized: BundleMessageType = serde_json::from_slice(&serialized).unwrap();

        match deserialized {
            BundleMessageType::Start {
                repo_id,
                file_name,
                total_size,
            } => {
                assert_eq!(repo_id, "repo123");
                assert_eq!(file_name, "repo.bundle");
                assert_eq!(total_size, 1024);
            }
            _ => panic!("Wrong message type"),
        }
    }
}
