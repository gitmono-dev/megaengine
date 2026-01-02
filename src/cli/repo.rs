use anyhow::Result;
use megaengine::{
    git::pack::{pull_repo_from_bundle, restore_repo_from_bundle},
    node::node_id::NodeId,
    repo::{self, repo::Repo, repo_id::RepoId},
    storage,
    util::timestamp_now,
};
use std::path::PathBuf;

pub async fn handle_repo_add(path: String, description: String) -> Result<()> {
    let kp = match storage::load_keypair() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("failed to load keypair: {}", e);
            tracing::info!("Run `auth init` first to generate keys");
            return Ok(());
        }
    };
    let node_id = NodeId::from_keypair(&kp);

    let root_bytes = match megaengine::git::git_repo::repo_root_commit_bytes(&path) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to read repo root commit: {}", e);
            println!("Ensure the provided path is a git repository with at least one commit");
            return Ok(());
        }
    };

    let repo_id = match RepoId::generate(root_bytes.as_slice(), &kp.verifying_key_bytes()) {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to generate RepoId: {}", e);
            return Ok(());
        }
    };

    let name = megaengine::git::git_repo::repo_name_space(&path);
    let desc = repo::repo::P2PDescription {
        creator: node_id.to_string(),
        name: name.clone(),
        description: description.clone(),
        timestamp: timestamp_now(),
    };

    let mut repo_obj =
        repo::repo::Repo::new(repo_id.to_string(), desc, PathBuf::from(path.clone()));

    // Read and populate refs from the git repository
    match megaengine::git::git_repo::read_repo_refs(&path) {
        Ok(refs) => {
            repo_obj.refs = refs;
            tracing::info!("Loaded {} refs from repository", repo_obj.refs.len());
        }
        Err(e) => {
            tracing::warn!("Failed to read refs from repository: {}", e);
        }
    }

    let mut manager = repo::repo_manager::RepoManager::new();
    match manager.register_repo(repo_obj).await {
        Ok(_) => tracing::info!("Repo {} added", repo_id),
        Err(e) => tracing::info!("Failed to add repo: {}", e),
    }
    Ok(())
}

pub async fn handle_repo_list() -> Result<()> {
    match storage::repo_model::list_repos().await {
        Ok(repos) => {
            if repos.is_empty() {
                println!("No repositories found");
            } else {
                println!("Repositories:");
                println!("{}", "─".repeat(120));
                for repo in repos {
                    print_repo_info(&repo).await;
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to list repos: {}", e);
            println!("Failed to list repositories: {}", e);
        }
    }
    Ok(())
}

async fn print_repo_info(repo: &Repo) {
    println!("  ID:          {}", repo.repo_id);
    println!("  Name:        {}", repo.p2p_description.name);
    println!("  Creator:     {}", repo.p2p_description.creator);
    println!("  Description: {}", repo.p2p_description.description);
    println!("  Path:        {}", repo.path.display());
    println!("  Bundle:      {}", repo.bundle.display());
    println!("  Timestamp:   {}", repo.p2p_description.timestamp);

    match megaengine::git::pack::extract_bundle_refs(&repo.bundle.to_string_lossy()) {
        Ok(local_refs) => {
            if local_refs.is_empty() {
                println!("  Refs:        (none)");
            } else {
                println!("  Refs:        ({} total)", local_refs.len());
                for (ref_name, commit) in &local_refs {
                    println!("    - {}: {}", ref_name, commit);
                }
            }

            // Check for updates if this is a local repo
            if !repo.path.as_os_str().is_empty() && repo.path.exists() {
                match megaengine::git::git_repo::read_repo_refs(repo.path.to_str().unwrap_or("")) {
                    Ok(current_refs) => {
                        // Compare current refs with local refs
                        if current_refs != local_refs {
                            println!("  Status:      ⚠️  HAS UPDATES");
                            println!("  Updated Refs: ({} total)", current_refs.len());
                            for (ref_name, commit) in &current_refs {
                                let local_commit = local_refs.get(ref_name);
                                if local_commit != Some(commit) {
                                    let indicator = if local_commit.is_none() {
                                        "NEW"
                                    } else {
                                        "CHANGED"
                                    };
                                    println!("    - {} {} : {}", indicator, ref_name, commit);
                                }
                            }
                        } else {
                            println!("  Status:      ✅ Up-to-date");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to check for updates: {}", e);
                        println!("  Status:      (failed to check: {})", e);
                    }
                }
            }
        }
        Err(e) => {
            println!("  Refs:        (failed to load: {})", e);
        }
    }

    println!("{}", "─".repeat(120));
}

pub async fn handle_repo_pull(repo_id: String) -> Result<()> {
    match storage::repo_model::load_repo_from_db(&repo_id).await {
        Ok(Some(repo)) => {
            // Check if repo has a local path
            if repo.path.as_os_str().is_empty() {
                tracing::error!("Repository {} has no local path", repo_id);
                println!("Error: Repository {} has no local path", repo_id);
                return Ok(());
            }
            // Check if bundle exists
            if repo.bundle.as_os_str().is_empty() {
                tracing::error!("Repository {} has no bundle available", repo_id);
                println!("Error: Repository {} has no bundle available", repo_id);
                return Ok(());
            }

            let result = pull_repo_from_bundle(
                repo.path.as_os_str().to_str().unwrap(),
                repo.bundle.as_os_str().to_str().unwrap(),
                "master",
            );

            match result {
                Ok(()) => {
                    tracing::info!("Repository {} fetched successfully from bundle", repo_id);
                    println!("✅ Repository updated successfully!");
                    println!("  Repository: {}", repo.p2p_description.name);
                    println!("  Path: {}", repo.path.display());
                }
                Err(e) => {
                    tracing::error!("Failed to spawn fetch task: {}", e);
                    println!("Error: Failed to spawn fetch task: {}", e);
                }
            }
        }
        Ok(None) => {
            tracing::error!("Repository {} not found in database", repo_id);
            println!("Error: Repository {} not found", repo_id);
        }
        Err(e) => {
            tracing::error!("Failed to query repository {}: {}", repo_id, e);
            println!("Error: Failed to query repository: {}", e);
        }
    }
    Ok(())
}

pub async fn handle_repo_clone(output: String, repo_id: String) -> Result<()> {
    match storage::repo_model::load_repo_from_db(&repo_id).await {
        Ok(Some(mut repo)) => {
            // Check if bundle exists
            if repo.bundle.as_os_str().is_empty() || repo.bundle.to_string_lossy().is_empty() {
                tracing::error!("Repository {} has no bundle available for cloning", repo_id);
                println!("Error: Repository {} has no bundle available", repo_id);
                return Ok(());
            }

            let bundle_path = repo.bundle.to_string_lossy().to_string();
            if !std::path::Path::new(&bundle_path).exists() {
                tracing::error!("Bundle file not found at path: {}", bundle_path);
                println!("Error: Bundle file not found at {}", bundle_path);
                return Ok(());
            }

            tracing::info!(
                "Cloning repository {} from bundle {} to {}",
                repo_id,
                bundle_path,
                output
            );

            match restore_repo_from_bundle(&bundle_path, &output).await {
                Ok(_) => {
                    tracing::info!("Repository {} cloned successfully to {}", repo_id, output);
                    println!("✅ Repository cloned successfully to {}", output);
                    println!("  Repository: {}", repo.p2p_description.name);
                    println!("  Creator: {}", repo.p2p_description.creator);
                    println!("  Description: {}", repo.p2p_description.description);

                    // Read and save refs from the cloned repository
                    match megaengine::git::git_repo::read_repo_refs(&output) {
                        Ok(refs) => {
                            tracing::info!("Loaded {} refs from cloned repository", refs.len());
                            // Save refs to the database
                            match storage::ref_model::batch_save_refs(&repo_id, &refs).await {
                                Ok(_) => {
                                    tracing::info!(
                                        "Refs saved to database for repository {}",
                                        repo_id
                                    );
                                    println!("  Refs: {} branches/tags", refs.len());
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to save refs to database: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to read refs from cloned repository: {}", e);
                        }
                    }

                    // Update repo path to the cloned location
                    repo.path = PathBuf::from(&output);
                    match storage::repo_model::save_repo_to_db(&repo).await {
                        Ok(_) => {
                            tracing::info!(
                                "Updated repo path to {} for repository {}",
                                output,
                                repo_id
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Failed to update repo path to database: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to clone repository: {}", e);
                    println!("Error: Failed to clone repository: {}", e);
                }
            }
        }
        Ok(None) => {
            tracing::error!("Repository {} not found in database", repo_id);
            println!("Error: Repository {} not found", repo_id);
        }
        Err(e) => {
            tracing::error!("Failed to query repository {}: {}", repo_id, e);
            println!("Error: Failed to query repository: {}", e);
        }
    }
    Ok(())
}

pub async fn handle_repo(action: crate::RepoAction) -> Result<()> {
    match action {
        crate::RepoAction::Add { path, description } => handle_repo_add(path, description).await,
        crate::RepoAction::List => handle_repo_list().await,
        crate::RepoAction::Pull { repo_id } => handle_repo_pull(repo_id).await,
        crate::RepoAction::Clone { output, repo_id } => handle_repo_clone(output, repo_id).await,
    }
}
