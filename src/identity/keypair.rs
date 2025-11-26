use anyhow::{anyhow, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyPair {
    pub signing_key: Option<SigningKey>,
    pub verifying_key: VerifyingKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self> {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        Ok(Self {
            signing_key: Some(signing_key),
            verifying_key,
        })
    }

    pub fn from_signing_key_bytes(bytes: [u8; 32]) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        Ok(Self {
            signing_key: Some(signing_key),
            verifying_key,
        })
    }

    pub fn from_verifying_key_bytes(verifying_key: [u8; 32]) -> Result<Self> {
        let verifying_key = VerifyingKey::from_bytes(&verifying_key)?;
        Ok(Self {
            signing_key: None,
            verifying_key,
        })
    }

    pub fn sign(&self, msg: &[u8]) -> Result<Signature> {
        if let Some(signing_key) = &self.signing_key {
            Ok(signing_key.sign(msg))
        } else {
            Err(anyhow!("no signing key"))
        }
    }

    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        self.verifying_key.verify(msg, sig).is_ok()
    }

    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        *self.verifying_key.as_bytes()
    }

    pub fn signing_key_bytes(&self) -> Result<[u8; 32]> {
        if let Some(signing_key) = &self.signing_key {
            Ok(*signing_key.as_bytes())
        } else {
            Err(anyhow!("no signing key"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_sign_verify() {
        let kp = KeyPair::generate().unwrap();
        let msg = b"hello world";
        let sig = kp.sign(msg).unwrap();
        assert!(kp.verify(msg, &sig));
    }

    #[test]
    fn test_export_and_import_signing_key() {
        let kp1 = KeyPair::generate().unwrap();
        let sk_bytes = kp1.signing_key_bytes().unwrap();
        let kp2 = KeyPair::from_signing_key_bytes(sk_bytes).unwrap();

        let msg = b"test message";
        let sig1 = kp1.sign(msg).unwrap();
        let sig2 = kp2.sign(msg).unwrap();
        assert_eq!(sig1.to_bytes(), sig2.to_bytes());
        assert_eq!(kp1.verifying_key.as_bytes(), kp2.verifying_key.as_bytes());
    }

    #[test]
    fn test_export_and_import_verifying_key() {
        let kp1 = KeyPair::generate().unwrap();
        let vk_bytes = kp1.verifying_key.as_bytes().clone();
        let kp2 = KeyPair::from_verifying_key_bytes(vk_bytes).unwrap();

        let msg = b"verify test";
        let sig = kp1.sign(msg).unwrap();

        assert!(kp2.verify(msg, &sig));
    }

    #[test]
    fn test_invalid_signature() {
        let kp1 = KeyPair::generate().unwrap();
        let kp2 = KeyPair::generate().unwrap();

        let msg = b"fake msg";
        let sig = kp1.sign(msg).unwrap();

        assert!(!kp2.verify(msg, &sig));
    }

    #[test]
    fn test_no_signing_key_error() {
        let kp =
            KeyPair::from_verifying_key_bytes(KeyPair::generate().unwrap().verifying_key_bytes())
                .unwrap();
        assert!(kp.sign(b"hi").is_err());
    }
}
