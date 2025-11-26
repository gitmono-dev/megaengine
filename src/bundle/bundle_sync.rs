use crate::node::node_id::NodeId;
use crate::repo::repo::Repo;
use crate::storage::repo_model;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{debug, info, warn};

use super::BundleService;

const SYNC_INTERVAL: Duration = Duration::from_secs(30);

/// 后台任务：定时检查和同步 external repos 的 bundle
pub async fn start_bundle_sync_task(bundle_service: Arc<Mutex<BundleService>>) {
    tokio::spawn(async move {
        let mut tick = interval(SYNC_INTERVAL);

        loop {
            tick.tick().await;

            debug!("Starting bundle sync check for external repos");

            // 查询所有 external repos
            match repo_model::list_repos().await {
                Ok(repos) => {
                    for repo in repos {
                        if repo.is_external && repo.bundle.as_os_str().is_empty() {
                            debug!(
                                "Found external repo without bundle: {} (creator: {})",
                                repo.repo_id, repo.p2p_description.creator
                            );

                            // 从creator节点请求bundle
                            if let Err(e) = request_bundle_from_owner(
                                &bundle_service,
                                &repo,
                                &repo.p2p_description.creator,
                            )
                            .await
                            {
                                warn!("Failed to request bundle for repo {}: {}", repo.repo_id, e);
                            }
                        } else if repo.is_external && !repo.bundle.as_os_str().is_empty() {
                            // Bundle 已存在，确保数据库已更新
                            debug!(
                                "External repo {} already has bundle: {}",
                                repo.repo_id,
                                repo.bundle.display()
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to list repos during sync check: {}", e);
                }
            }
        }
    });
}

/// 从仓库所有者请求 bundle
async fn request_bundle_from_owner(
    bundle_service: &Arc<Mutex<BundleService>>,
    repo: &Repo,
    owner_node_id_str: &str,
) -> Result<()> {
    info!(
        "Requesting bundle for repo {} from node {}",
        repo.repo_id, owner_node_id_str
    );

    // 解析所有者的 NodeId
    let owner_node_id = NodeId::from_string(owner_node_id_str)?;

    // 通过 BundleService 的 request_bundle 发送 Request 消息
    let service = bundle_service.lock().await;
    service
        .request_bundle(&owner_node_id, &repo.repo_id)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {}
