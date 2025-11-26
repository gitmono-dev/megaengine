use anyhow::Result;
use git2::Repository;
use std::path::Path;
use std::process::Command;

/// Pack a git repository into a single file using git bundle
/// This creates a bundle file that contains all branches and commits
///
/// # Arguments
/// * `repo_path` - Path to the git repository to pack
/// * `output_path` - Path where the bundle file will be created
///
/// # Example
/// ```ignore
/// pack_repo_bundle("/path/to/repo", "/tmp/repo.bundle")?;
/// ```
pub fn pack_repo_bundle(repo_path: &str, output_path: &str) -> Result<()> {
    // 检查并创建 output_path 的目录
    if let Some(parent_dir) = Path::new(output_path).parent() {
        if !parent_dir.as_os_str().is_empty() {
            std::fs::create_dir_all(parent_dir)
                .map_err(|e| anyhow::anyhow!("failed to create output directory: {}", e))?;
        }
    }

    let repo = Repository::open(repo_path)
        .map_err(|e| anyhow::anyhow!("failed to open git repo: {}", e))?;

    // Get all branches to include in the bundle
    let mut branch_refs = Vec::new();
    let branches = repo
        .branches(None)
        .map_err(|e| anyhow::anyhow!("failed to list branches: {}", e))?;

    for branch_result in branches {
        let (branch, _) =
            branch_result.map_err(|e| anyhow::anyhow!("failed to get branch: {}", e))?;
        if let Ok(Some(name_str)) = branch.name() {
            branch_refs.push(name_str.to_string());
        }
    }

    // If no branches found, try to get HEAD
    if branch_refs.is_empty()
        && repo.head().is_ok() {
            branch_refs.push("HEAD".to_string());
        }

    if branch_refs.is_empty() {
        return Err(anyhow::anyhow!("no branches found to bundle"));
    }

    // Use git bundle command to create the bundle
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("bundle")
        .arg("create")
        .arg(output_path);

    for branch_ref in &branch_refs {
        cmd.arg(branch_ref);
    }

    let output = cmd
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute git bundle: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git bundle failed: {}", stderr));
    }

    Ok(())
}

/// Restore a git repository from a bundle file
/// This creates a new repository by cloning from the bundle
///
/// # Arguments
/// * `bundle_path` - Path to the bundle file
/// * `output_path` - Path where the new repository will be created
///
/// # Example
/// ```ignore
/// restore_repo_from_bundle("/tmp/repo.bundle", "/path/to/new/repo").await?;
/// ```
pub async fn restore_repo_from_bundle(bundle_path: &str, output_path: &str) -> Result<()> {
    // 检查 bundle 文件是否存在
    if !Path::new(bundle_path).exists() {
        return Err(anyhow::anyhow!("bundle file not found: {}", bundle_path));
    }

    // 检查输出目录是否已存在
    let output_dir = Path::new(output_path);
    if output_dir.exists() {
        return Err(anyhow::anyhow!(
            "output directory already exists: {}",
            output_path
        ));
    }

    // 创建输出目录的父目录
    if let Some(parent_dir) = output_dir.parent() {
        if !parent_dir.as_os_str().is_empty() {
            std::fs::create_dir_all(parent_dir)
                .map_err(|e| anyhow::anyhow!("failed to create output directory: {}", e))?;
        }
    }

    // 在线程中执行 git clone，避免阻塞 async 运行时
    let bundle_path = bundle_path.to_string();
    let output_path = output_path.to_string();

    tokio::task::spawn_blocking(move || {
        // 使用 git clone 从 bundle 恢复仓库
        let output = Command::new("git")
            .arg("clone")
            .arg(&bundle_path)
            .arg(&output_path)
            .output()
            .map_err(|e| anyhow::anyhow!("failed to execute git clone: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git clone from bundle failed: {}", stderr));
        }

        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("failed to spawn bundle restore task: {}", e))?
}
