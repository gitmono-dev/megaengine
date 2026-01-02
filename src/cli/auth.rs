use anyhow::Result;
use megaengine::storage;

pub async fn handle_auth() -> Result<()> {
    let kp_path = storage::keypair_path();
    if kp_path.exists() {
        tracing::info!(
            "Keypair already exists at {:?}; skipping generation",
            kp_path
        );
    } else {
        tracing::info!("Generating new keypair...");
        let kp = megaengine::identity::keypair::KeyPair::generate()?;
        storage::save_keypair(&kp)?;
        tracing::info!("Keypair saved to {:?}", storage::keypair_path());
    }
    Ok(())
}
