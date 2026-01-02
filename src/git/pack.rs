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
    if branch_refs.is_empty() && repo.head().is_ok() {
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

/// Extract refs information from a git bundle file
/// Uses `git bundle list-heads` to get the refs and their commit hashes
///
/// # Arguments
/// * `bundle_path` - Path to the bundle file
///
/// # Returns
/// A HashMap of ref_name -> commit_hash
///
/// # Example
/// ```ignore
/// let refs = extract_bundle_refs("/tmp/repo.bundle")?;
/// ```
pub fn extract_bundle_refs(bundle_path: &str) -> Result<std::collections::HashMap<String, String>> {
    // 检查 bundle 文件是否存在
    if !Path::new(bundle_path).exists() {
        return Err(anyhow::anyhow!("bundle file not found: {}", bundle_path));
    }

    // 使用 git bundle list-heads 获取 bundle 中的 refs
    let output = Command::new("git")
        .arg("bundle")
        .arg("list-heads")
        .arg(bundle_path)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute git bundle list-heads: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git bundle list-heads failed: {}", stderr));
    }

    // 解析输出，格式为: <commit_hash> <ref_name>
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut refs = std::collections::HashMap::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let commit_hash = parts[0].to_string();
            let ref_name = parts[1].to_string();
            refs.insert(ref_name, commit_hash);
        }
    }

    Ok(refs)
}

/// Pull updates from a git bundle file into an existing repository
/// This updates the existing repository with commits from the bundle
///
/// # Arguments
/// * `repo_path` - Path to the existing git repository
/// * `bundle_path` - Path to the bundle file
/// * `branch` - Branch name to pull from the bundle (e.g., "master" or "refs/heads/master")
///
/// # Example
/// ```ignore
/// pull_repo_from_bundle("/path/to/repo", "/tmp/repo.bundle", "master")?;
/// ```
pub fn pull_repo_from_bundle(repo_path: &str, bundle_path: &str, branch: &str) -> Result<()> {
    // 检查 bundle 文件是否存在
    if !Path::new(bundle_path).exists() {
        return Err(anyhow::anyhow!("bundle file not found: {}", bundle_path));
    }

    // 检查仓库是否存在
    if !Path::new(repo_path).exists() {
        return Err(anyhow::anyhow!("repository not found: {}", repo_path));
    }

    let _repo = Repository::open(repo_path)
        .map_err(|e| anyhow::anyhow!("failed to open git repo: {}", e))?;

    // 构建分支引用名称，确保格式正确
    let _ref_spec = if branch.starts_with("refs/") {
        branch.to_string()
    } else if branch == "HEAD" {
        "HEAD".to_string()
    } else {
        format!("refs/heads/{}", branch)
    };

    // 使用 git pull 从 bundle 拉取更新
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("pull")
        .arg(bundle_path)
        .arg(branch)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute git pull: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git pull from bundle failed: {}", stderr));
    }

    Ok(())
}
