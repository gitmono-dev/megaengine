use anyhow::Result;
use git2::{BranchType, Repository, Sort};

pub fn repo_root_commit_bytes(path: &str) -> Result<Vec<u8>> {
    let repo =
        Repository::open(path).map_err(|e| anyhow::anyhow!("failed to open git repo: {}", e))?;
    let mut revwalk = repo
        .revwalk()
        .map_err(|e| anyhow::anyhow!("revwalk error: {}", e))?;
    revwalk
        .push_head()
        .map_err(|e| anyhow::anyhow!("push_head failed: {}", e))?;
    let _ = revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE);

    if let Some(entry) = revwalk.next() {
        let oid = entry.map_err(|e| anyhow::anyhow!("revwalk entry error: {}", e))?;
        return Ok(oid.as_bytes().to_vec());
    }

    Err(anyhow::anyhow!("no commits found in repo"))
}

pub fn repo_name_space(path: &str) -> String {
    let repo = match Repository::open(path) {
        Ok(repo) => repo,
        Err(_) => {
            return "".to_string();
        }
    };
    let path = repo.path();

    if let Some(name) = path.parent().and_then(|p| p.file_name()) {
        return name.to_string_lossy().to_string();
    }
    "".to_string()
}

/// Read all refs (branches and tags) from a git repository
pub fn read_repo_refs(path: &str) -> Result<std::collections::HashMap<String, String>> {
    let repo =
        Repository::open(path).map_err(|e| anyhow::anyhow!("failed to open git repo: {}", e))?;

    let mut refs = std::collections::HashMap::new();

    // Read local branches (refs/heads/*)
    let branches = repo
        .branches(Some(BranchType::Local))
        .map_err(|e| anyhow::anyhow!("failed to read branches: {}", e))?;

    for branch_result in branches {
        let (branch, _branch_type) =
            branch_result.map_err(|e| anyhow::anyhow!("failed to read branch: {}", e))?;
        if let Ok(Some(name)) = branch.name() {
            if let Some(oid) = branch.get().target() {
                let ref_name = format!("refs/heads/{}", name);
                refs.insert(ref_name, oid.to_string());
            }
        }
    }

    // Read tags (refs/tags/*)
    let tag_names = repo
        .tag_names(None)
        .map_err(|e| anyhow::anyhow!("failed to read tags: {}", e))?;

    for tag_name in tag_names.iter().flatten() {
        if let Ok(reference) = repo.find_reference(&format!("refs/tags/{}", tag_name)) {
            if let Some(oid) = reference.target() {
                refs.insert(format!("refs/tags/{}", tag_name), oid.to_string());
            }
        }
    }

    Ok(refs)
}

pub fn get_latest_commit_time(path: &str) -> Result<i64> {
    let repo =
        Repository::open(path).map_err(|e| anyhow::anyhow!("failed to open git repo: {}", e))?;
    let head = repo
        .head()
        .map_err(|e| anyhow::anyhow!("failed to get HEAD: {}", e))?;
    let commit = head
        .peel_to_commit()
        .map_err(|e| anyhow::anyhow!("failed to peel to commit: {}", e))?;
    Ok(commit.time().seconds())
}
