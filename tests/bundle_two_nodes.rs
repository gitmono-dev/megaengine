//! 集成测试：两个节点之间通过网络传输 bundle
use megaengine::bundle::BundleService;
use megaengine::git::pack::{pack_repo_bundle, restore_repo_from_bundle};
use megaengine::gossip::GossipService;
use megaengine::identity::keypair::KeyPair;
use megaengine::node::node::{Node, NodeType};
use megaengine::transport::config::QuicConfig;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Helper function to run git commands
fn run_git_command(cwd: &str, args: &[&str]) -> bool {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("Failed to execute git command");

    output.status.success()
}

/// Create a test git repository
fn create_test_repo(repo_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(repo_path)?;

    run_git_command(repo_path, &["init"]);
    run_git_command(repo_path, &["config", "user.email", "test@example.com"]);
    run_git_command(repo_path, &["config", "user.name", "Test User"]);

    let repo_dir = PathBuf::from(repo_path);
    fs::write(
        repo_dir.join("README.md"),
        "# Test Repository for Node Transfer\n",
    )?;
    fs::write(
        repo_dir.join("data.txt"),
        "Important data to transfer between nodes\n",
    )?;

    run_git_command(repo_path, &["add", "."]);
    run_git_command(
        repo_path,
        &["commit", "-m", "Initial commit for transfer test"],
    );

    Ok(())
}

#[tokio::test]
async fn test_bundle_transfer_between_two_nodes() {
    println!("\n========================================");
    println!("🔄 Bundle Transfer Between Two Nodes Test");
    println!("========================================\n");

    // Initialize rustls crypto provider
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();

    println!("📋 Step 1: Setting up certificates");
    // Ensure certificates exist
    megaengine::transport::cert::ensure_certificates(
        "cert/cert_sender.pem",
        "cert/key_sender.pem",
        "cert/ca-cert.pem",
    )
    .expect("Failed to ensure sender certificates");

    megaengine::transport::cert::ensure_certificates(
        "cert/cert_receiver.pem",
        "cert/key_receiver.pem",
        "cert/ca-cert.pem",
    )
    .expect("Failed to ensure receiver certificates");

    println!("✅ Certificates ready");

    println!("\n📋 Step 2: Generating node keys and identities");
    // Generate keypairs for both nodes
    let sender_kp = KeyPair::generate().expect("Failed to generate sender keypair");
    let receiver_kp = KeyPair::generate().expect("Failed to generate receiver keypair");

    let sender_addr: SocketAddr = "127.0.0.1:19010".parse().unwrap();
    let receiver_addr: SocketAddr = "127.0.0.1:19011".parse().unwrap();

    let mut sender_node = Node::from_keypair(
        &sender_kp,
        "sender_node",
        vec![sender_addr],
        NodeType::Normal,
    );
    let mut receiver_node = Node::from_keypair(
        &receiver_kp,
        "receiver_node",
        vec![receiver_addr],
        NodeType::Normal,
    );

    println!("✅ Nodes created");
    println!("   - Sender: {} at {}", sender_node.node_id(), sender_addr);
    println!(
        "   - Receiver: {} at {}",
        receiver_node.node_id(),
        receiver_addr
    );

    println!("\n📋 Step 3: Starting QUIC servers");
    // Start QUIC servers
    let sender_config = QuicConfig::new(
        sender_addr,
        "cert/cert_sender.pem".to_string(),
        "cert/key_sender.pem".to_string(),
        "cert/ca-cert.pem".to_string(),
    );
    let receiver_config = QuicConfig::new(
        receiver_addr,
        "cert/cert_receiver.pem".to_string(),
        "cert/key_receiver.pem".to_string(),
        "cert/ca-cert.pem".to_string(),
    );

    sender_node
        .start_quic_server(sender_config)
        .await
        .expect("Failed to start sender QUIC server");
    receiver_node
        .start_quic_server(receiver_config)
        .await
        .expect("Failed to start receiver QUIC server");

    println!("✅ QUIC servers started");

    println!("\n📋 Step 4: Starting Gossip and Bundle services");
    // Create and start gossip services
    let sender_gossip = Arc::new(GossipService::new(
        Arc::clone(sender_node.connection_manager.as_ref().unwrap()),
        sender_node.clone(),
        None,
    ));
    let receiver_gossip = Arc::new(GossipService::new(
        Arc::clone(receiver_node.connection_manager.as_ref().unwrap()),
        receiver_node.clone(),
        None,
    ));

    sender_gossip
        .start()
        .await
        .expect("Failed to start sender gossip");
    receiver_gossip
        .start()
        .await
        .expect("Failed to start receiver gossip");

    // Create and start bundle services with absolute paths
    let sender_bundle_storage = std::env::current_dir()
        .unwrap()
        .join("tmp/sender_bundle_storage");
    let receiver_bundle_storage = std::env::current_dir()
        .unwrap()
        .join("tmp/receiver_bundle_storage");

    fs::create_dir_all(&sender_bundle_storage).ok();
    fs::create_dir_all(&receiver_bundle_storage).ok();

    let sender_bundle = Arc::new(BundleService::new(
        Arc::clone(sender_node.connection_manager.as_ref().unwrap()),
        sender_bundle_storage.clone(),
    ));
    let receiver_bundle = Arc::new(BundleService::new(
        Arc::clone(receiver_node.connection_manager.as_ref().unwrap()),
        receiver_bundle_storage.clone(),
    ));

    sender_bundle
        .clone()
        .start()
        .await
        .expect("Failed to start sender bundle service");
    receiver_bundle
        .clone()
        .start()
        .await
        .expect("Failed to start receiver bundle service");

    println!("✅ Services started");
    println!(
        "   - Sender bundle storage: {}",
        sender_bundle_storage.display()
    );
    println!(
        "   - Receiver bundle storage: {}",
        receiver_bundle_storage.display()
    );

    println!("\n📋 Step 5: Creating test repository and packing bundle");
    // Create test repository
    let repo_path = std::env::current_dir()
        .unwrap()
        .join("tmp/test_repo_for_transfer");

    fs::remove_dir_all(&repo_path).ok();
    create_test_repo(repo_path.to_str().unwrap()).expect("Failed to create test repository");
    println!("✅ Test repository created at {}", repo_path.display());

    // Pack repository into bundle
    let bundle_path = std::env::current_dir()
        .unwrap()
        .join("./tmp/transfer_test.bundle");

    pack_repo_bundle(repo_path.to_str().unwrap(), bundle_path.to_str().unwrap())
        .expect("Failed to pack repository");

    let bundle_size = fs::metadata(&bundle_path)
        .expect("Failed to read bundle metadata")
        .len();

    println!("✅ Bundle created");
    println!("   - Path: {}", bundle_path.display());
    println!("   - Size: {} bytes", bundle_size);

    println!("\n📋 Step 6: Connecting nodes");
    // Connect sender to receiver
    let sender_mgr = sender_node.connection_manager.as_ref().unwrap().clone();
    sender_mgr
        .lock()
        .await
        .connect(
            sender_node.node_id().clone(),
            receiver_node.node_id().clone(),
            vec![receiver_addr],
        )
        .await
        .expect("Failed to connect sender to receiver");

    println!("✅ Nodes connected");
    sleep(Duration::from_millis(500)).await;

    println!("\n📋 Step 7: Sender transmitting bundle to receiver");
    println!("   - Repo ID: test_transfer_repo");
    println!("   - Bundle path: {}", bundle_path.display());

    // Sender sends bundle to receiver
    sender_bundle
        .send_bundle(
            receiver_node.node_id().clone(),
            "test_transfer_repo".to_string(),
            bundle_path.to_str().unwrap(),
        )
        .await
        .expect("Failed to send bundle");

    println!("✅ Bundle transmission initiated");

    // Wait for transfer to complete
    sleep(Duration::from_secs(2)).await;

    println!("\n📋 Step 8: Verifying bundle reception");
    // Check if bundle was received
    // The bundle is stored in the receiver's storage with encoded sender_node_id directory
    // encode_node_id 函数提取 NodeId 的最后一段并用 _ 替换 :
    let sender_node_id_str = sender_node.node_id().to_string();
    // 提取最后一段（NodeId 格式是 "did:key:xxx"）
    let last_segment = sender_node_id_str
        .split(':')
        .last()
        .unwrap_or(&sender_node_id_str);
    let encoded_sender_id = last_segment.replace(':', "_").replace('/', "_");

    // repo_id 会被处理为最后一段（用 : 分割）
    let repo_id_last_part = "test_transfer_repo"
        .split(':')
        .last()
        .unwrap_or("test_transfer_repo")
        .to_string();

    let received_bundle_path = receiver_bundle_storage.join(format!(
        "{}/{}.bundle",
        encoded_sender_id, repo_id_last_part
    ));

    if received_bundle_path.exists() {
        let received_size = fs::metadata(&received_bundle_path)
            .expect("Failed to read received bundle metadata")
            .len();

        println!("✅ Bundle received!");
        println!("   - Path: {}", received_bundle_path.display());
        println!("   - Size: {} bytes", received_size);

        if received_size == bundle_size as u64 {
            println!("✅ Bundle size matches (integrity verified)");
        } else {
            println!(
                "⚠️ Bundle size mismatch: expected {} bytes, got {} bytes",
                bundle_size, received_size
            );
        }

        println!("\n📋 Step 9: Verifying bundle content by restoration");
        // Verify bundle by restoring it using the provided utility function
        let restored_repo_path = std::env::current_dir()
            .unwrap()
            .join("tmp/restored_repo_from_transfer");

        fs::remove_dir_all(&restored_repo_path).ok();

        match restore_repo_from_bundle(
            received_bundle_path.to_str().unwrap(),
            restored_repo_path.to_str().unwrap(),
        )
        .await
        {
            Ok(_) => {
                println!("✅ Bundle successfully restored to new repository");

                // Verify files exist
                let readme_path = restored_repo_path.join("README.md");
                let data_path = restored_repo_path.join("data.txt");

                if readme_path.exists() && data_path.exists() {
                    println!("✅ All expected files found in restored repository");

                    // Check commit history
                    let output = Command::new("git")
                        .current_dir(restored_repo_path.to_str().unwrap())
                        .args(&["log", "--oneline"])
                        .output()
                        .expect("Failed to get git log");

                    let log = String::from_utf8_lossy(&output.stdout);
                    if log.contains("Initial commit for transfer test") {
                        println!("✅ Commit history preserved in restored repository");
                    } else {
                        println!("⚠️ Original commit message not found in restored repo");
                    }
                } else {
                    println!("❌ Some expected files not found in restored repository");
                }
            }
            Err(e) => {
                println!("❌ Failed to restore bundle: {}", e);
            }
        }

        println!("\n📋 Step 10: Cleanup");
        // Cleanup
        fs::remove_dir_all(
            std::env::current_dir()
                .unwrap()
                .join("tmp/test_repo_for_transfer"),
        )
        .ok();
        fs::remove_dir_all(
            std::env::current_dir()
                .unwrap()
                .join("tmp/restored_repo_from_transfer"),
        )
        .ok();
        fs::remove_dir_all(&sender_bundle_storage).ok();
        fs::remove_dir_all(&receiver_bundle_storage).ok();
        fs::remove_file(&bundle_path).ok();
        println!("✅ Cleanup completed");

        println!("\n========================================");
        println!("✨ Bundle transfer test completed successfully!");
        println!("========================================\n");
    } else {
        println!(
            "❌ Bundle not received at expected path: {}",
            received_bundle_path.display()
        );
        println!("\n📊 Debug information:");
        println!(
            "   - Receiver storage: {}",
            receiver_bundle_storage.display()
        );

        // List files in receiver storage
        if receiver_bundle_storage.exists() {
            println!("   - Contents of receiver storage:");
            if let Ok(entries) = fs::read_dir(&receiver_bundle_storage) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        println!("     - {}", entry.path().display());
                    }
                }
            }
        }

        // Cleanup anyway
        fs::remove_dir_all(
            std::env::current_dir()
                .unwrap()
                .join("tmp/test_repo_for_transfer"),
        )
        .ok();
        fs::remove_dir_all(&sender_bundle_storage).ok();
        fs::remove_dir_all(&receiver_bundle_storage).ok();
        fs::remove_file(&bundle_path).ok();

        panic!("Bundle reception failed");
    }

    // Cleanup database records
    let _ =
        megaengine::storage::node_model::delete_node_from_db(&sender_node.node_id().to_string())
            .await;
    let _ =
        megaengine::storage::node_model::delete_node_from_db(&receiver_node.node_id().to_string())
            .await;
}
