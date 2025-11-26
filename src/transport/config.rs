use anyhow::Result;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, IdleTimeout, ServerConfig, TransportConfig, VarInt};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

pub const ALPN_QUIC_HTTP: &[&[u8]] = &[b"h3"];

/// 用于开发/测试环境的服务器证书验证器
/// 跳过所有服务器证书验证，允许自签名证书和不同的 CA
#[derive(Debug)]
struct NoServerCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoServerCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // 跳过验证，允许任何证书
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

#[derive(Clone, Debug)]
pub struct QuicConfig {
    pub bind_addr: SocketAddr,
    pub cert_path: String,
    pub key_path: String,
    pub ca_cert_path: String,
}

impl QuicConfig {
    pub fn new(
        bind_addr: SocketAddr,
        cert_path: String,
        key_path: String,
        ca_cert_path: String,
    ) -> Self {
        QuicConfig {
            bind_addr,
            cert_path,
            key_path,
            ca_cert_path,
        }
    }

    /// 获取服务器配置
    /// 注意：不验证客户端证书，仅适用于开发/测试环境
    /// 生产环境应该使用正确的 CA 证书验证
    pub fn get_server_config(&self) -> Result<ServerConfig> {
        let (certs, key) = self.get_certificate_from_file()?;

        let mut server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth() // 不验证客户端证书
            .with_single_cert(certs, key)?;
        server_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
        server_crypto.max_early_data_size = u32::MAX;

        let mut server_config =
            ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));

        let mut transport_config = TransportConfig::default();
        transport_config.max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(300_000))));
        transport_config.keep_alive_interval(Some(Duration::from_secs(30)));
        server_config.transport_config(Arc::new(transport_config));

        Ok(server_config)
    }

    /// 获取客户端配置
    /// 注意：使用不验证服务器证书的配置，仅适用于开发/测试环境
    /// 生产环境应该使用正确的 CA 证书验证
    pub fn get_client_config(&self) -> Result<ClientConfig> {
        let (certs, key) = self.get_certificate_from_file()?;

        // 创建一个不验证服务器证书的客户端配置
        // 这对于开发/测试环境很有用，当每个节点都有独立的证书时
        let mut client_crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerCertificateVerification))
            .with_client_auth_cert(certs, key)?;

        client_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
        client_crypto.enable_early_data = false;
        let client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(client_crypto)?));
        Ok(client_config)
    }

    /// 从文件读取证书和密钥
    pub fn get_certificate_from_file(
        &self,
    ) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
        let cert_file = File::open(self.cert_path.as_str())?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs = rustls_pemfile::certs(&mut cert_reader).collect::<std::io::Result<Vec<_>>>()?;

        if certs.is_empty() {
            return Err(anyhow::anyhow!("No certificates found in PEM file"));
        }

        let file = File::open(self.key_path.as_str())?;
        let mut reader = BufReader::new(file);

        // 尝试读取PKCS8格式的私钥
        if let Some(key) = rustls_pemfile::private_key(&mut reader)? {
            return Ok((certs, key));
        }

        // 如果PKCS8格式读取失败，重新读取文件尝试其他格式
        let file = File::open(self.key_path.as_str())?;
        let mut reader = BufReader::new(file);

        // 尝试读取所有可能的私钥格式
        let keys =
            rustls_pemfile::pkcs8_private_keys(&mut reader).collect::<std::io::Result<Vec<_>>>()?;

        if !keys.is_empty() {
            return Ok((certs, PrivateKeyDer::Pkcs8(keys[0].clone_key())));
        }
        Err(anyhow::anyhow!("No key found in PEM file"))
    }

    /// 从文件读取 CA 证书
    pub fn get_ca_certificate_from_file(&self) -> Result<CertificateDer<'static>> {
        let file = File::open(self.ca_cert_path.as_str())?;
        let mut reader = BufReader::new(file);

        let certs = rustls_pemfile::certs(&mut reader).collect::<std::io::Result<Vec<_>>>()?;

        if certs.is_empty() {
            return Err(anyhow::anyhow!("No certificates found in CA PEM file"));
        }

        Ok(certs[0].clone())
    }
}
