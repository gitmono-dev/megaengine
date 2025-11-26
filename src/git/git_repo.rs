use anyhow::Result;
use git2::{Repository, Sort};

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
