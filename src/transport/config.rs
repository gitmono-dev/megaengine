use anyhow::{Context, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, IdleTimeout, ServerConfig, TransportConfig, VarInt};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

pub const ALPN_QUIC_HTTP: &[&[u8]] = &[b"h3"];

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
    pub fn get_server_config(&self) -> Result<ServerConfig> {
        let (certs, key) = self.get_certificate_from_file()?;

        let mut roots = rustls::RootCertStore::empty();
        let ca_cert = self.get_ca_certificate_from_file()?;
        roots.add(ca_cert)?;

        let client_verifier = WebPkiClientVerifier::builder(roots.into())
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut server_crypto = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
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
    pub fn get_client_config(&self) -> Result<ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        let ca_cert = self.get_ca_certificate_from_file()?;
        roots.add(ca_cert)?;
        let (certs, key) = self.get_certificate_from_file()?;

        // let mut client_crypto = rustls::ClientConfig::builder()
        //     .with_root_certificates(roots)
        //     .with_no_client_auth();

        let mut client_crypto = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
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
