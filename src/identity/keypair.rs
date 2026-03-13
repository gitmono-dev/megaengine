#![allow(deprecated)]
use anyhow::{anyhow, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use curve25519_dalek::{edwards::CompressedEdwardsY, montgomery::MontgomeryPoint, scalar::Scalar};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use rand_core::RngCore;
use serde::Deserialize;
use serde::Serialize;
use sha2::{Digest, Sha256};

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

    /// Encrypt a message for a specific recipient (identified by their Ed25519 VerifyingKey)
    /// Returns: Ephemeral_PK (32) + Nonce (12) + Ciphertext (N)
    pub fn encrypt_to_node(&self, recipient_vk: &VerifyingKey, message: &[u8]) -> Result<Vec<u8>> {
        let _rng = OsRng;

        // 1. Convert Recipient Ed25519 PK -> X25519 PK (Montgomery)
        let recipient_ed_y = CompressedEdwardsY::from_slice(recipient_vk.as_bytes())?;
        let recipient_ed_point = recipient_ed_y
            .decompress()
            .ok_or(anyhow!("Invalid Public Key Point"))?;
        let recipient_mont_point = recipient_ed_point.to_montgomery();

        // 2. Generate Ephemeral Keypair
        let mut scalar_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut scalar_bytes);
        let ephemeral_scalar = Scalar::from_bytes_mod_order(scalar_bytes);
        let ephemeral_point = MontgomeryPoint::mul_base(&ephemeral_scalar);

        // 3. Keep Ephemeral Public Key
        let ephemeral_pk_bytes = ephemeral_point.to_bytes();

        // 4. Calculate Shared Secret: ephemeral_secret * recipient_public
        let shared_secret_point = ephemeral_scalar * recipient_mont_point;
        let shared_secret_bytes = shared_secret_point.to_bytes();

        // 5. Derive Encryption Key (Hash)
        let mut hasher = Sha256::new();
        hasher.update(shared_secret_bytes);
        hasher.update(ephemeral_pk_bytes);
        hasher.update(recipient_mont_point.to_bytes());
        let key_hash = hasher.finalize();

        let key = chacha20poly1305::Key::from_slice(&key_hash);
        let cipher = ChaCha20Poly1305::new(key);

        // 6. Encrypt
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, message)
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;

        // 7. Pack: EphemeralPK (32) + Nonce (12) + Ciphertext
        let mut result = Vec::with_capacity(32 + 12 + ciphertext.len());
        result.extend_from_slice(&ephemeral_pk_bytes);
        result.extend_from_slice(nonce);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt a message addressed to this keypair
    pub fn decrypt_message(&self, payload: &[u8]) -> Result<Vec<u8>> {
        if payload.len() < 32 + 12 {
            return Err(anyhow!("Message too short"));
        }

        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or(anyhow!("No private key available for decryption"))?;

        // 1. My Secret Key Conversion
        let mut hasher = sha2::Sha512::new();
        hasher.update(signing_key.as_bytes());
        let h = hasher.finalize();

        let mut clamped = [0u8; 32];
        clamped.copy_from_slice(&h[0..32]);
        clamped[0] &= 248;
        clamped[31] &= 127;
        clamped[31] |= 64;

        let my_scalar = Scalar::from_bits(clamped);

        // 2. Parse Payload
        let ephemeral_pk_bytes = &payload[0..32];
        let nonce_bytes = &payload[32..44];
        let ciphertext = &payload[44..];

        let ephemeral_point = MontgomeryPoint(ephemeral_pk_bytes.try_into()?);

        // 3. Calculate Shared Secret: my_secret * ephemeral_public
        let shared_secret_point = my_scalar * ephemeral_point;
        let shared_secret_bytes = shared_secret_point.to_bytes();

        // 4. Derive Key
        let my_ed_y = CompressedEdwardsY::from_slice(self.verifying_key.as_bytes())?;
        let my_ed_point = my_ed_y
            .decompress()
            .ok_or(anyhow!("Invalid My Public Key"))?;
        let my_mont_point = my_ed_point.to_montgomery();

        let mut hasher = Sha256::new();
        hasher.update(shared_secret_bytes);
        hasher.update(ephemeral_pk_bytes);
        hasher.update(my_mont_point.to_bytes());
        let key_hash = hasher.finalize();

        let key = chacha20poly1305::Key::from_slice(&key_hash);
        let cipher = ChaCha20Poly1305::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        // 5. Decrypt
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("Decryption failed: {}", e))?;

        Ok(plaintext)
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
        let vk_bytes = *kp1.verifying_key.as_bytes();
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
