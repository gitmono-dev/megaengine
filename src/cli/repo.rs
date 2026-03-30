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
    let language = detect_language(&path);

    // Calculate size: prefer .git directory size (repository data) over working tree size
    let path_p = std::path::Path::new(&path);
    let git_dir = path_p.join(".git");
    let size = if git_dir.exists() {
        calculate_directory_size(&git_dir)
    } else {
        0
    };

    // Try to get latest git commit time, fallback to now if failed (e.g. empty repo)
    let latest_commit_at = match megaengine::git::git_repo::get_latest_commit_time(&path) {
        Ok(t) => t,
        Err(_) => timestamp_now(),
    };

    let desc = repo::repo::P2PDescription {
        creator: node_id.to_string(),
        name: name.clone(),
        description: description.clone(),
        language: language.clone(),
        latest_commit_at,
        size,
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
        Ok(_) => {
            tracing::info!("Repo {} added", repo_id);
            println!("✅ Repository added successfully!");
            println!("  ID:     {}", repo_id);
            println!("  Name:   {}", name);
        }
        Err(e) => {
            tracing::error!("Failed to add repo: {}", e);
            eprintln!("❌ Failed to add repository: {}", e);
        }
    }
    Ok(())
}

fn detect_language(path: &str) -> String {
    use std::collections::HashMap;
    use std::fs;

    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let mut stack = vec![PathBuf::from(path)];
    let mut files_scanned = 0;

    while let Some(dir) = stack.pop() {
        if files_scanned > 2000 {
            break;
        } // limit scanning

        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with('.')
                        || name == "target"
                        || name == "node_modules"
                        || name == "dist"
                        || name == "build"
                    {
                        continue;
                    }
                    if stack.len() < 50 {
                        stack.push(path);
                    }
                } else if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    *ext_counts.entry(ext.to_lowercase()).or_insert(0) += 1;
                    files_scanned += 1;
                }
            }
        }
    }

    let mut lang_stats = HashMap::new();
    for (ext, count) in ext_counts {
        let lang = match ext.as_str() {
            "rs" => "Rust",
            "go" => "Go",
            "py" => "Python",
            "js" => "JavaScript",
            "ts" | "tsx" => "TypeScript",
            "java" => "Java",
            "c" | "h" => "C",
            "cpp" | "hpp" | "cc" | "cxx" => "C++",
            "cs" => "C#",
            "rb" => "Ruby",
            "php" => "PHP",
            "html" => "HTML",
            "css" | "scss" | "less" => "CSS",
            "swift" => "Swift",
            "kt" | "kts" => "Kotlin",
            "scala" => "Scala",
            "lua" => "Lua",
            "sh" | "bash" | "zsh" => "Shell",
            "sql" => "SQL",
            "md" => "Markdown",
            "json" | "yaml" | "yml" | "toml" | "xml" => "Config/Data",
            _ => continue,
        };
        *lang_stats.entry(lang).or_insert(0) += count;
    }

    // 找出数量最多的语言，排除配置类
    lang_stats
        .into_iter()
        .filter(|(l, _)| *l != "Config/Data" && *l != "Markdown")
        .max_by_key(|&(_, count)| count)
        .map(|(lang, _)| lang.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn calculate_directory_size(path: &std::path::Path) -> u64 {
    use std::fs;

    const MAX_DEPTH: usize = 64;
    const MAX_ENTRIES: u64 = 200_000;
    const MAX_TOTAL_SIZE: u64 = 20 * 1024 * 1024 * 1024; // 20 GiB

    fn walk(path: &std::path::Path, depth: usize, entries_seen: &mut u64, total: &mut u64) {
        if depth > MAX_DEPTH || *entries_seen >= MAX_ENTRIES || *total >= MAX_TOTAL_SIZE {
            return;
        }

        let Ok(entries) = fs::read_dir(path) else {
            return;
        };

        for entry in entries.flatten() {
            if *entries_seen >= MAX_ENTRIES || *total >= MAX_TOTAL_SIZE {
                break;
            }

            *entries_seen += 1;
            let p = entry.path();

            // Never follow symlinks to avoid cycles and unbounded traversal.
            let Ok(meta) = fs::symlink_metadata(&p) else {
                continue;
            };

            let file_type = meta.file_type();
            if file_type.is_symlink() {
                continue;
            }

            if file_type.is_file() {
                *total = total.saturating_add(meta.len());
            } else if file_type.is_dir() {
                walk(&p, depth + 1, entries_seen, total);
            }
        }
    }

    let mut entries_seen = 0;
    let mut total = 0;
    walk(path, 0, &mut entries_seen, &mut total);
    total
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub async fn handle_repo_list() -> Result<()> {
    match storage::repo_model::list_repos().await {
        Ok(repos) => {
            if repos.is_empty() {
                println!("No repositories found.");
            } else {
                println!("Found {} repositories:", repos.len());
                println!("{}", "─".repeat(60));
                for repo in repos {
                    print_repo_info(&repo).await;
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to list repos: {}", e);
            eprintln!("❌ Failed to list repositories: {}", e);
        }
    }
    Ok(())
}

async fn print_repo_info(repo: &Repo) {
    println!("📦 Repo: {}", repo.p2p_description.name);
    println!("   ID:          {}", repo.repo_id);
    println!("   Creator:     {}", repo.p2p_description.creator);
    println!("   Language:    {}", repo.p2p_description.language);
    if repo.p2p_description.latest_commit_at > 0 {
        if let Some(dt) = chrono::DateTime::from_timestamp(repo.p2p_description.latest_commit_at, 0)
        {
            let local = dt.with_timezone(&chrono::Local);
            println!("   Updated:     {}", local.format("%Y-%m-%d %H:%M:%S"));
        }
    }
    if repo.p2p_description.size > 0 {
        println!(
            "   Size:        {}",
            format_bytes(repo.p2p_description.size)
        );
    }
    if !repo.p2p_description.description.is_empty() {
        println!("   Description: {}", repo.p2p_description.description);
    }
    println!("   Path:        {}", repo.path.display());
    // Bundle path only shown if it exists, to reduce clutter
    if !repo.bundle.as_os_str().is_empty() {
        println!("   Bundle:      {}", repo.bundle.display());
    }

    // Status check logic...
    if repo.bundle.as_os_str().is_empty() {
        println!("   Refs:        (bundle not set)");
    } else {
        match megaengine::git::pack::extract_bundle_refs(&repo.bundle.to_string_lossy()) {
            Ok(local_refs) => {
                let ref_count = local_refs.len();

                // Check if up-to-date with local path
                let mut status_msg = "✅ Synced".to_string();
                let mut updates = Vec::new();

                if !repo.path.as_os_str().is_empty() && repo.path.exists() {
                    match megaengine::git::git_repo::read_repo_refs(
                        repo.path.to_str().unwrap_or(""),
                    ) {
                        Ok(current_refs) => {
                            if current_refs != local_refs {
                                status_msg = "⚠️  Out of Sync".to_string();
                                for (ref_name, commit) in &current_refs {
                                    if local_refs.get(ref_name) != Some(commit) {
                                        updates.push(format!("{} -> {}", ref_name, &commit[0..7]));
                                    }
                                }
                            }
                        }
                        Err(_) => status_msg = "❓ Unknown (Check Failed)".to_string(),
                    }
                }

                println!("   Refs:        {} branches/tags", ref_count);
                println!("   Status:      {}", status_msg);

                if !updates.is_empty() {
                    println!("   Updates:     {} pending changes", updates.len());
                    for update in updates.iter().take(3) {
                        println!("     - {}", update);
                    }
                    if updates.len() > 3 {
                        println!("     - ... and {} more", updates.len() - 3);
                    }
                }
            }
            Err(_) => println!("   Refs:        (error loading bundle)"),
        }
    }

    println!("{}", "─".repeat(60));
}

pub async fn handle_repo_pull(repo_id: String) -> Result<()> {
    println!("🔄 Pulling repository {}...", repo_id);
    match storage::repo_model::load_repo_from_db(&repo_id).await {
        Ok(Some(repo)) => {
            // Check if repo has a local path
            if repo.path.as_os_str().is_empty() {
                tracing::error!("Repository {} has no local path", repo_id);
                eprintln!(
                    "❌ Error: Repository {} has no local path configured.",
                    repo_id
                );
                return Ok(());
            }
            // Check if bundle exists
            if repo.bundle.as_os_str().is_empty() {
                tracing::error!("Repository {} has no bundle available", repo_id);
                eprintln!("❌ Error: Repository {} has no bundle available.", repo_id);
                return Ok(());
            }

            let path_str = match repo.path.as_os_str().to_str() {
                Some(s) => s,
                None => {
                    eprintln!("❌ Error: Local path is not valid UTF-8.");
                    return Ok(());
                }
            };

            let bundle_str = match repo.bundle.as_os_str().to_str() {
                Some(s) => s,
                None => {
                    eprintln!("❌ Error: Bundle path is not valid UTF-8.");
                    return Ok(());
                }
            };

            let result = pull_repo_from_bundle(path_str, bundle_str, "master");

            match result {
                Ok(()) => {
                    tracing::info!("Repository {} fetched successfully from bundle", repo_id);
                    println!("✅ Repository updated successfully!");
                    println!("   Name: {}", repo.p2p_description.name);
                    println!("   Path: {}", repo.path.display());
                }
                Err(e) => {
                    tracing::error!("Failed to spawn fetch task: {}", e);
                    eprintln!("❌ Failed to update repository: {}", e);
                }
            }
        }
        Ok(None) => {
            tracing::error!("Repository {} not found in database", repo_id);
            eprintln!("❌ Error: Repository {} not found.", repo_id);
        }
        Err(e) => {
            tracing::error!("Failed to query repository {}: {}", repo_id, e);
            eprintln!("❌ Database error: {}", e);
        }
    }
    Ok(())
}

pub async fn handle_repo_clone(output: String, repo_id: String) -> Result<()> {
    println!("📥 Cloning repository {}...", repo_id);
    match storage::repo_model::load_repo_from_db(&repo_id).await {
        Ok(Some(mut repo)) => {
            // Check if bundle exists
            if repo.bundle.as_os_str().is_empty() || repo.bundle.to_string_lossy().is_empty() {
                tracing::error!("Repository {} has no bundle available for cloning", repo_id);
                eprintln!("❌ Error: Repository {} has no bundle available.", repo_id);
                return Ok(());
            }

            let bundle_path = repo.bundle.to_string_lossy().to_string();
            if !std::path::Path::new(&bundle_path).exists() {
                tracing::error!("Bundle file not found at path: {}", bundle_path);
                eprintln!("❌ Error: Bundle file not found at {}", bundle_path);
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
                    println!("✅ Repository cloned successfully!");
                    println!("   Name:        {}", repo.p2p_description.name);
                    println!("   Creator:     {}", repo.p2p_description.creator);
                    println!("   Description: {}", repo.p2p_description.description);
                    println!("   Path:        {}", output);

                    // Read and save refs from the cloned repository
                    match megaengine::git::git_repo::read_repo_refs(&output) {
                        Ok(refs) => {
                            tracing::info!("Loaded {} refs from cloned repository", refs.len());
                            // Save refs to the database
                            match storage::ref_model::batch_save_refs(&repo_id, &refs).await {
                                Ok(_) => {
                                    println!(
                                        "   Refs:        {} branches/tags imported",
                                        refs.len()
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to save refs to database: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to read refs from cloned repository: {}", e);
                            println!("   ⚠️ Warning: Failed to read refs from cloned repo: {}", e);
                        }
                    }

                    // Update repo path to the cloned location
                    repo.path = PathBuf::from(&output);
                    match storage::repo_model::save_repo_to_db(&repo).await {
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("Failed to update repo path to database: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to clone repository: {}", e);
                    eprintln!("❌ Failed to clone repository: {}", e);
                }
            }
        }
        Ok(None) => {
            tracing::error!("Repository {} not found in database", repo_id);
            eprintln!("❌ Error: Repository {} not found.", repo_id);
        }
        Err(e) => {
            tracing::error!("Failed to query repository {}: {}", repo_id, e);
            eprintln!("❌ Database error: {}", e);
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
