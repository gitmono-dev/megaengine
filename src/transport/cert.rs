use anyhow::{anyhow, Result};
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, KeyPair};
use std::fs;
use std::path::Path;

/// Generate a CA certificate and save to files.
pub fn generate_ca_cert(ca_cert_path: &str, ca_key_path: &str) -> Result<Certificate> {
    // Check if CA certificate already exists
    if Path::new(ca_cert_path).exists() && Path::new(ca_key_path).exists() {
        tracing::info!(
            "CA certificate already exists at {} and {}",
            ca_cert_path,
            ca_key_path
        );
        // Return a dummy cert since we can't reconstruct it from PEM
        // But files exist so they'll be used by other functions
        return Err(anyhow!("CA cert exists, but cannot reconstruct from PEM"));
    }

    // Create cert directory if needed
    if let Some(parent) = Path::new(ca_cert_path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    tracing::info!("Generating CA certificate...");

    // Generate a keypair for CA
    let keypair =
        KeyPair::generate().map_err(|e| anyhow!("Failed to generate CA keypair: {}", e))?;

    // Create CA certificate parameters
    let mut params = CertificateParams::new(vec![])
        .map_err(|e| anyhow!("Failed to create CA certificate params: {}", e))?;

    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    // Set CA subject name
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "MegaEngine CA");
    dn.push(DnType::OrganizationName, "MegaEngine");
    dn.push(DnType::CountryName, "CN");
    params.distinguished_name = dn;

    // Generate self-signed CA certificate
    let ca_cert = params
        .self_signed(&keypair)
        .map_err(|e| anyhow!("Failed to generate CA certificate: {}", e))?;

    // Save CA certificate
    let ca_cert_pem = ca_cert.pem();
    fs::write(ca_cert_path, ca_cert_pem)?;
    tracing::info!("CA certificate written to {}", ca_cert_path);

    // Save CA private key
    let ca_key_pem = keypair.serialize_pem();
    fs::write(ca_key_path, ca_key_pem)?;
    tracing::info!("CA private key written to {}", ca_key_path);

    Ok(ca_cert)
}

/// Generate a server certificate signed by CA.
pub fn generate_server_cert(
    cert_path: &str,
    key_path: &str,
    ca_cert_obj: &Certificate,
    ca_key_path: &str,
) -> Result<()> {
    // Check if certificate already exists
    if Path::new(cert_path).exists() && Path::new(key_path).exists() {
        tracing::info!(
            "Server certificate already exists at {} and {}",
            cert_path,
            key_path
        );
        return Ok(());
    }

    // Create cert directory if needed
    if let Some(parent) = Path::new(cert_path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    tracing::info!("Generating server certificate signed by CA...");

    // Read CA key
    let ca_key_pem = fs::read_to_string(ca_key_path)?;

    // Parse CA key
    let ca_keypair =
        KeyPair::from_pem(&ca_key_pem).map_err(|e| anyhow!("Failed to parse CA key: {}", e))?;

    // Generate server keypair
    let server_keypair =
        KeyPair::generate().map_err(|e| anyhow!("Failed to generate server keypair: {}", e))?;

    // Create server certificate parameters with SANs
    let mut params = CertificateParams::new(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "0.0.0.0".to_string(),
    ])
    .map_err(|e| anyhow!("Failed to create server certificate params: {}", e))?;

    // Set server subject name
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "localhost");
    dn.push(DnType::OrganizationName, "MegaEngine");
    dn.push(DnType::CountryName, "CN");
    params.distinguished_name = dn;

    // Sign server certificate with CA key
    // signed_by expects (server_keypair, ca_cert_obj, ca_keypair)
    let server_cert = params
        .signed_by(&server_keypair, ca_cert_obj, &ca_keypair)
        .map_err(|e| anyhow!("Failed to generate server certificate: {}", e))?;

    // Save server certificate
    let cert_pem = server_cert.pem();
    fs::write(cert_path, cert_pem)?;
    tracing::info!("Server certificate written to {}", cert_path);

    // Save server private key
    let key_pem = server_keypair.serialize_pem();
    fs::write(key_path, key_pem)?;
    tracing::info!("Server private key written to {}", key_path);

    Ok(())
}

/// Ensure certificates exist: generate CA once, then generate different server certs.
pub fn ensure_certificates(cert_path: &str, key_path: &str, ca_cert_path: &str) -> Result<()> {
    // Derive CA key path from CA cert path
    let ca_key_path = ca_cert_path.replace(".pem", "-key.pem");

    // Check if both cert and key exist - if only one exists, something went wrong, regenerate both
    let cert_exists = Path::new(cert_path).exists();
    let key_exists = Path::new(key_path).exists();

    if cert_exists != key_exists {
        // Mismatch - delete both and regenerate
        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }

    // Generate CA certificate if needed (only once)
    let ca_cert = match generate_ca_cert(ca_cert_path, &ca_key_path) {
        Ok(cert) => cert,
        Err(_) => {
            // CA already exists - need to reconstruct it from files for signing
            tracing::info!("CA certificate exists, reconstructing from files");

            let ca_key_pem = fs::read_to_string(&ca_key_path)?;
            let keypair = KeyPair::from_pem(&ca_key_pem)?;
            let mut params = CertificateParams::new(vec![])?;
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, "MegaEngine CA");
            dn.push(DnType::OrganizationName, "MegaEngine");
            dn.push(DnType::CountryName, "CN");
            params.distinguished_name = dn;
            params.self_signed(&keypair)?
        }
    };

    // Generate server certificate signed by CA
    // If server cert and key don't both exist, regenerate them
    if !cert_exists || !key_exists {
        generate_server_cert(cert_path, key_path, &ca_cert, &ca_key_path)?;
    }

    Ok(())
}
