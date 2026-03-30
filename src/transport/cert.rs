use anyhow::{anyhow, Result};
use libvault::utils::cert::Certificate;
use openssl::pkey::{PKey, Private};
use openssl::x509::X509;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

fn build_ca_certificate() -> Result<libvault::utils::cert::CertBundle> {
    let mut ca = Certificate {
        is_ca: true,
        key_type: "rsa".to_string(),
        key_bits: 2048,
        not_after: SystemTime::now() + Duration::from_secs(60 * 60 * 24 * 3650),
        ..Default::default()
    };

    ca.to_cert_bundle(None, None)
        .map_err(|e| anyhow!("Failed to generate CA certificate: {}", e))
}

fn build_server_certificate(ca_cert: &X509, ca_key: &PKey<Private>) -> Result<libvault::utils::cert::CertBundle> {
    let mut cert = Certificate {
        dns_sans: vec!["localhost".to_string()],
        ip_sans: vec!["127.0.0.1".to_string(), "0.0.0.0".to_string()],
        is_ca: false,
        key_type: "rsa".to_string(),
        key_bits: 2048,
        not_after: SystemTime::now() + Duration::from_secs(60 * 60 * 24 * 3650),
        ..Default::default()
    };

    cert.to_cert_bundle(Some(ca_cert), Some(ca_key))
        .map_err(|e| anyhow!("Failed to generate server certificate: {}", e))
}

/// Generate a CA certificate and save to files.
pub fn generate_ca_cert(ca_cert_path: &str, ca_key_path: &str) -> Result<()> {
    // Check if CA certificate already exists
    if Path::new(ca_cert_path).exists() && Path::new(ca_key_path).exists() {
        tracing::info!(
            "CA certificate already exists at {} and {}",
            ca_cert_path,
            ca_key_path
        );
        return Ok(());
    }

    // Create cert directory if needed
    if let Some(parent) = Path::new(ca_cert_path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    tracing::info!("Generating CA certificate...");

    let ca_cert = build_ca_certificate()?;

    // Save CA certificate
    let ca_cert_pem = ca_cert.certificate.to_pem()?;
    fs::write(ca_cert_path, ca_cert_pem)?;
    tracing::info!("CA certificate written to {}", ca_cert_path);

    // Save CA private key
    let ca_key_pem = ca_cert.private_key.private_key_to_pem_pkcs8()?;
    fs::write(ca_key_path, ca_key_pem)?;
    tracing::info!("CA private key written to {}", ca_key_path);

    Ok(())
}

/// Generate a server certificate signed by CA.
pub fn generate_server_cert(
    cert_path: &str,
    key_path: &str,
    ca_cert_obj: &X509,
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
    let ca_key_pem = fs::read(ca_key_path)?;

    // Parse CA key
    let ca_key = PKey::private_key_from_pem(&ca_key_pem)
        .map_err(|e| anyhow!("Failed to parse CA key: {}", e))?;

    let server_cert = build_server_certificate(ca_cert_obj, &ca_key)?;

    // Save server certificate
    let cert_pem = server_cert.certificate.to_pem()?;
    fs::write(cert_path, cert_pem)?;
    tracing::info!("Server certificate written to {}", cert_path);

    // Save server private key
    let key_pem = server_cert.private_key.private_key_to_pem_pkcs8()?;
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
    generate_ca_cert(ca_cert_path, &ca_key_path)?;

    let ca_cert_pem = fs::read(ca_cert_path)?;
    let ca_cert = X509::from_pem(&ca_cert_pem).map_err(|e| anyhow!("Failed to parse CA cert: {}", e))?;

    // Generate server certificate signed by CA
    // If server cert and key don't both exist, regenerate them
    if !cert_exists || !key_exists {
        generate_server_cert(cert_path, key_path, &ca_cert, &ca_key_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(prefix: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("megaengine-{}-{}", prefix, ts))
    }

    #[test]
    fn test_ensure_certificates_generates_files() {
        let dir = test_dir("cert-generate");
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        let ca_cert_path = dir.join("ca-cert.pem");
        let ca_key_path = dir.join("ca-cert-key.pem");

        let result = ensure_certificates(
            cert_path.to_str().expect("cert path utf8"),
            key_path.to_str().expect("key path utf8"),
            ca_cert_path.to_str().expect("ca cert path utf8"),
        );

        assert!(result.is_ok());
        assert!(cert_path.exists());
        assert!(key_path.exists());
        assert!(ca_cert_path.exists());
        assert!(ca_key_path.exists());

        let cert_pem = std::fs::read(&cert_path).expect("read cert pem");
        let key_pem = std::fs::read(&key_path).expect("read key pem");
        let ca_cert_pem = std::fs::read(&ca_cert_path).expect("read ca cert pem");

        assert!(X509::from_pem(&cert_pem).is_ok());
        assert!(PKey::private_key_from_pem(&key_pem).is_ok());
        assert!(X509::from_pem(&ca_cert_pem).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_certificates_is_idempotent_when_files_exist() {
        let dir = test_dir("cert-idempotent");
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        let ca_cert_path = dir.join("ca-cert.pem");

        ensure_certificates(
            cert_path.to_str().expect("cert path utf8"),
            key_path.to_str().expect("key path utf8"),
            ca_cert_path.to_str().expect("ca cert path utf8"),
        )
        .expect("first ensure");

        let cert_before = std::fs::read(&cert_path).expect("read cert before");
        let key_before = std::fs::read(&key_path).expect("read key before");

        ensure_certificates(
            cert_path.to_str().expect("cert path utf8"),
            key_path.to_str().expect("key path utf8"),
            ca_cert_path.to_str().expect("ca cert path utf8"),
        )
        .expect("second ensure");

        let cert_after = std::fs::read(&cert_path).expect("read cert after");
        let key_after = std::fs::read(&key_path).expect("read key after");

        assert_eq!(cert_before, cert_after);
        assert_eq!(key_before, key_after);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_certificates_recovers_from_missing_key() {
        let dir = test_dir("cert-recover");
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        let ca_cert_path = dir.join("ca-cert.pem");

        ensure_certificates(
            cert_path.to_str().expect("cert path utf8"),
            key_path.to_str().expect("key path utf8"),
            ca_cert_path.to_str().expect("ca cert path utf8"),
        )
        .expect("first ensure");

        std::fs::remove_file(&key_path).expect("remove key file");
        assert!(cert_path.exists());
        assert!(!key_path.exists());

        ensure_certificates(
            cert_path.to_str().expect("cert path utf8"),
            key_path.to_str().expect("key path utf8"),
            ca_cert_path.to_str().expect("ca cert path utf8"),
        )
        .expect("recovery ensure");

        assert!(cert_path.exists());
        assert!(key_path.exists());

        let cert_pem = std::fs::read(&cert_path).expect("read cert pem");
        let key_pem = std::fs::read(&key_path).expect("read key pem");
        assert!(X509::from_pem(&cert_pem).is_ok());
        assert!(PKey::private_key_from_pem(&key_pem).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
