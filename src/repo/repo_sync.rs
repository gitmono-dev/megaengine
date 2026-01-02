use crate::git::git_repo::read_repo_refs;
use crate::storage::{ref_model, repo_model};
use anyhow::Result;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, info, warn};

const REPO_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// 后台任务：定时检查本地 repos 的 refs 是否有更新
pub async fn start_repo_sync_task() {
    tokio::spawn(async move {
        let mut tick = interval(REPO_CHECK_INTERVAL);

        loop {
            tick.tick().await;

            debug!("Starting repo refs check");

            // 查询所有 repos
            match repo_model::list_repos().await {
                Ok(repos) => {
                    for repo in repos {
                        // 只检查本地 repos (is_external=false)
                        if !repo.is_external {
                            if let Err(e) = check_and_update_repo_refs(&repo).await {
                                warn!("Failed to check refs for repo {}: {}", repo.repo_id, e);
                            }
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

/// 检查仓库的 refs 是否有更新，如果有则更新数据库
async fn check_and_update_repo_refs(repo: &crate::repo::repo::Repo) -> Result<()> {
    let repo_path = repo.path.to_string_lossy().to_string();

    // 从 git 仓库读取最新的 refs
    let current_refs = read_repo_refs(&repo_path)?;

    // 检查是否有变化
    if ref_model::has_refs_changed(&repo.repo_id, &current_refs).await? {
        info!(
            "Detected refs change in local repo {}, updating database",
            repo.repo_id
        );

        // 更新 refs 到数据库
        ref_model::batch_save_refs(&repo.repo_id, &current_refs).await?;

        // 这里后续可以扩展：
        // 1. 重新生成 bundle
        // 2. 广播 repo announcement 告知其他节点有更新
        // TODO: trigger bundle generation and gossip announcement

        debug!(
            "Successfully updated refs for repo {} ({} refs)",
            repo.repo_id,
            current_refs.len()
        );
    } else {
        debug!("No changes detected in repo {}", repo.repo_id);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_repo_sync_task_spawns() {
        // 只测试任务能否正常启动，不测试实际功能
        start_repo_sync_task().await;
        // 任务已在后台运行，测试通过
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
