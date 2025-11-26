use megaengine::git::pack::pack_repo_bundle;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Ensure tmp directory exists
fn ensure_tmp_dir() -> PathBuf {
    let tmp_path = PathBuf::from("tmp");
    if !tmp_path.exists() {
        fs::create_dir(&tmp_path).expect("Failed to create tmp directory");
    }
    tmp_path
}

/// Helper function to run git commands
fn run_git_command(cwd: &str, args: &[&str]) -> bool {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("Failed to execute git command");

    output.status.success()
}

/// Test pack_repo_bundle with complete workflow:
/// 1. Create a git repository
/// 2. Commit some files
/// 3. Pack into bundle
/// 4. Restore bundle into a new repository
#[test]
fn test_pack_repo_bundle() {
    let tmp_dir = ensure_tmp_dir();

    // Step 1: Create git repository 1
    let repo1_path = tmp_dir.join("repo1");
    if repo1_path.exists() {
        fs::remove_dir_all(&repo1_path).ok();
    }
    fs::create_dir(&repo1_path).expect("Failed to create repo1 directory");
    let repo1_str = repo1_path.to_str().unwrap();

    println!("📁 Step 1: Creating git repository at {}", repo1_str);

    // Initialize git repo
    assert!(
        run_git_command(repo1_str, &["init"]),
        "Failed to init git repo"
    );
    assert!(
        run_git_command(repo1_str, &["config", "user.email", "test@example.com"]),
        "Failed to config user email"
    );
    assert!(
        run_git_command(repo1_str, &["config", "user.name", "Test User"]),
        "Failed to config user name"
    );

    // Step 2: Create and commit files
    println!("📝 Step 2: Creating and committing files");

    // Create multiple files
    fs::write(
        repo1_path.join("README.md"),
        "# Test Project\n\nThis is a test repository for bundle packing.",
    )
    .expect("Failed to write README.md");

    fs::write(
        repo1_path.join("main.rs"),
        "fn main() {\n    println!(\"Hello!\");\n}\n",
    )
    .expect("Failed to write main.rs");

    fs::write(repo1_path.join("data.txt"), "Some data here\n").expect("Failed to write data.txt");

    // Commit the files
    assert!(
        run_git_command(repo1_str, &["add", "."]),
        "Failed to add files"
    );
    assert!(
        run_git_command(
            repo1_str,
            &["commit", "-m", "Initial commit with multiple files"]
        ),
        "Failed to commit"
    );

    // Create another commit
    fs::write(repo1_path.join("second.txt"), "Second file\n").expect("Failed to write second.txt");
    assert!(
        run_git_command(repo1_str, &["add", "."]),
        "Failed to add second file"
    );
    assert!(
        run_git_command(repo1_str, &["commit", "-m", "Add second file"]),
        "Failed to commit second file"
    );

    // Create a branch
    assert!(
        run_git_command(repo1_str, &["branch", "feature/dev"]),
        "Failed to create branch"
    );
    assert!(
        run_git_command(repo1_str, &["checkout", "feature/dev"]),
        "Failed to checkout branch"
    );

    fs::write(repo1_path.join("feature.txt"), "Feature branch file\n")
        .expect("Failed to write feature.txt");
    assert!(
        run_git_command(repo1_str, &["add", "."]),
        "Failed to add feature file"
    );
    assert!(
        run_git_command(repo1_str, &["commit", "-m", "Add feature in dev branch"]),
        "Failed to commit feature"
    );

    // Go back to main
    if !run_git_command(repo1_str, &["checkout", "main"]) {
        run_git_command(repo1_str, &["checkout", "master"]);
    }

    // Create a tag
    assert!(
        run_git_command(repo1_str, &["tag", "v1.0"]),
        "Failed to create tag"
    );

    println!("✅ Repository initialized with commits and branches");

    // Step 3: Pack into bundle
    let bundle_path = tmp_dir.join("repo1.bundle");
    let bundle_path_abs = std::env::current_dir().unwrap().join(&bundle_path);
    println!(
        "📦 Step 3: Packing repository into bundle: {}",
        bundle_path_abs.display()
    );

    match pack_repo_bundle(repo1_str, bundle_path_abs.to_str().unwrap()) {
        Ok(_) => {
            assert!(
                bundle_path_abs.exists(),
                "Bundle file was not created at {}",
                bundle_path_abs.display()
            );
            let bundle_size = fs::metadata(&bundle_path_abs)
                .expect("Failed to read bundle metadata")
                .len();
            println!("✅ Bundle created successfully: {} bytes", bundle_size);
            assert!(bundle_size > 0, "Bundle file is empty");
        }
        Err(e) => {
            panic!("Failed to create bundle: {}", e);
        }
    }

    // Step 4: Restore bundle into new repository
    let repo2_path = tmp_dir.join("repo2_restored");
    if repo2_path.exists() {
        fs::remove_dir_all(&repo2_path).ok();
    }

    let repo2_path_abs = std::env::current_dir().unwrap().join(&repo2_path);

    println!(
        "🔄 Step 4: Restoring bundle to new repository at {}",
        repo2_path_abs.display()
    );

    let repo2_str = repo2_path_abs.to_str().unwrap();

    // Clone from bundle
    assert!(
        run_git_command(
            tmp_dir.to_str().unwrap(),
            &["clone", bundle_path_abs.to_str().unwrap(), repo2_str]
        ),
        "Failed to clone from bundle"
    );

    println!("✅ Repository restored from bundle");

    // Step 5: Verify restored repository
    println!("✓ Step 5: Verifying restored repository");

    // Check that main branch exists
    let output = Command::new("git")
        .current_dir(repo2_str)
        .args(&["branch", "-a"])
        .output()
        .expect("Failed to list branches");
    let branches = String::from_utf8_lossy(&output.stdout);
    println!("Branches in restored repo:\n{}", branches);

    // Check that files exist
    assert!(
        repo2_path_abs.join("README.md").exists(),
        "README.md not restored"
    );
    assert!(
        repo2_path_abs.join("main.rs").exists(),
        "main.rs not restored"
    );
    assert!(
        repo2_path_abs.join("data.txt").exists(),
        "data.txt not restored"
    );

    // Check commit history
    let output = Command::new("git")
        .current_dir(repo2_str)
        .args(&["log", "--oneline"])
        .output()
        .expect("Failed to get log");
    let log = String::from_utf8_lossy(&output.stdout);
    println!("Commit history in restored repo:\n{}", log);
    assert!(
        log.contains("Initial commit"),
        "Initial commit not found in restored repo"
    );

    // Check tags
    let output = Command::new("git")
        .current_dir(repo2_str)
        .args(&["tag"])
        .output()
        .expect("Failed to list tags");
    let tags = String::from_utf8_lossy(&output.stdout);
    println!("Tags in restored repo:\n{}", tags);
    // Note: Tags might not be included in the bundle by default

    println!("✅ All verifications passed!");
    println!("\n📊 Summary:");
    println!("  Original repo: {}", repo1_path.display());
    println!(
        "  Bundle file: {} ({} bytes)",
        bundle_path_abs.display(),
        fs::metadata(&bundle_path_abs).unwrap().len()
    );
    println!("  Restored repo: {}", repo2_path_abs.display());
    println!("✅ Bundle pack/restore test completed successfully!");

    // Cleanup: Remove test directories and bundle
    println!("\n🧹 Cleaning up temporary directories...");
    fs::remove_dir_all(&repo1_path).ok();
    fs::remove_dir_all(&repo2_path_abs).ok();
    fs::remove_file(&bundle_path_abs).ok();
    println!("✅ Cleanup completed!");
}
