use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

use crate::error::{Error, Result};

type HmacSha256 = Hmac<Sha256>;

pub struct Compressor;
pub struct PacketCipher;
pub struct AuthSigner;

impl Compressor {
    pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(data)
            .map_err(|e| Error::Transport(format!("Compression failed: {}", e)))?;
        encoder
            .finish()
            .map_err(|e| Error::Transport(format!("Compression finish failed: {}", e)))
    }

    pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = GzDecoder::new(data);
        let mut result = Vec::new();
        decoder
            .read_to_end(&mut result)
            .map_err(|e| Error::Transport(format!("Decompression failed: {}", e)))?;
        Ok(result)
    }
}

impl PacketCipher {
    pub fn encrypt(data: &[u8], secret: &str) -> Result<Vec<u8>> {
        if secret.is_empty() {
            return Err(Error::Transport(
                "encryption secret cannot be empty".to_string(),
            ));
        }

        let key = derive_key(secret);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| Error::Transport(format!("cipher init failed: {}", e)))?;

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted = cipher
            .encrypt(nonce, data)
            .map_err(|e| Error::Transport(format!("encrypt failed: {}", e)))?;

        let mut out = Vec::with_capacity(12 + encrypted.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&encrypted);
        Ok(out)
    }

    pub fn decrypt(data: &[u8], secret: &str) -> Result<Vec<u8>> {
        if secret.is_empty() {
            return Err(Error::Transport(
                "encryption secret cannot be empty".to_string(),
            ));
        }
        if data.len() < 12 {
            return Err(Error::Transport("encrypted payload too short".to_string()));
        }

        let key = derive_key(secret);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| Error::Transport(format!("cipher init failed: {}", e)))?;
        let nonce = Nonce::from_slice(&data[..12]);
        cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| Error::Transport(format!("decrypt failed: {}", e)))
    }
}

impl AuthSigner {
    /// Generate HMAC-SHA256 signature: hex(HMAC(token, message)).
    /// Used to sign privilege_key = HMAC(token, timestamp) so the raw token
    /// is never transmitted over the wire.
    pub fn sign(token: &str, message: &str) -> String {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(token.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        let result = mac.finalize();
        hex_encode(&result.into_bytes())
    }

    /// Verify that `signature_hex` == HMAC(token, message).
    pub fn verify(token: &str, message: &str, signature_hex: &str) -> bool {
        let expected = Self::sign(token, message);
        // Constant-time comparison via byte-by-byte (both are hex strings of same length)
        if expected.len() != signature_hex.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in expected.bytes().zip(signature_hex.bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn derive_key(secret: &str) -> [u8; 32] {
    let digest = Sha256::digest(secret.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    out
}

/// Session-scoped cipher that caches the derived AES-256-GCM key.
/// Use this in hot paths (relay_stcp, UDP proxy) instead of `PacketCipher`
/// to avoid re-deriving the key via SHA-256 on every packet.
pub struct SessionCipher {
    cipher: Aes256Gcm,
}

impl SessionCipher {
    pub fn new(secret: &str) -> Self {
        let key = derive_key(secret);
        let cipher = Aes256Gcm::new_from_slice(&key).expect("valid AES-256 key length");
        Self { cipher }
    }

    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted = self
            .cipher
            .encrypt(nonce, data)
            .map_err(|e| Error::Transport(format!("encrypt failed: {}", e)))?;

        let mut out = Vec::with_capacity(12 + encrypted.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&encrypted);
        Ok(out)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Err(Error::Transport("encrypted payload too short".to_string()));
        }
        let nonce = Nonce::from_slice(&data[..12]);
        self.cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| Error::Transport(format!("decrypt failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress() {
        let original = b"Hello, World! This is a test message that should be compressed.";
        let compressed = Compressor::compress(original).unwrap();
        let decompressed = Compressor::decompress(&compressed).unwrap();

        assert_eq!(original.to_vec(), decompressed);
        assert!(!compressed.is_empty());
    }

    #[test]
    fn test_compress_empty() {
        let original = b"";
        let compressed = Compressor::compress(original).unwrap();
        let decompressed = Compressor::decompress(&compressed).unwrap();

        assert_eq!(original.to_vec(), decompressed);
    }

    #[test]
    fn test_encrypt_decrypt() {
        let secret = "test-secret";
        let original = b"hello secure world";
        let encrypted = PacketCipher::encrypt(original, secret).unwrap();
        let decrypted = PacketCipher::decrypt(&encrypted, secret).unwrap();
        assert_eq!(original.to_vec(), decrypted);
    }

    #[test]
    fn test_session_cipher_encrypt_decrypt() {
        let cipher = SessionCipher::new("session-secret");
        let original = b"hello from session cipher";
        let encrypted = cipher.encrypt(original).unwrap();
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(original.to_vec(), decrypted);
    }

    #[test]
    fn test_session_cipher_compatible_with_packet_cipher() {
        let secret = "shared-secret";
        let original = b"cross-compat test";
        // SessionCipher encrypt, PacketCipher decrypt
        let cipher = SessionCipher::new(secret);
        let encrypted = cipher.encrypt(original).unwrap();
        let decrypted = PacketCipher::decrypt(&encrypted, secret).unwrap();
        assert_eq!(original.to_vec(), decrypted);
    }

    #[test]
    fn test_decrypt_with_wrong_secret() {
        let original = b"hello secure world";
        let encrypted = PacketCipher::encrypt(original, "secret-a").unwrap();
        assert!(PacketCipher::decrypt(&encrypted, "secret-b").is_err());
    }

    #[test]
    fn test_hmac_sign_and_verify() {
        let token = "my-secret-token";
        let message = "1700000000";
        let sig = AuthSigner::sign(token, message);
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64); // SHA256 = 32 bytes = 64 hex chars
        assert!(AuthSigner::verify(token, message, &sig));
    }

    #[test]
    fn test_hmac_rejects_wrong_token() {
        let sig = AuthSigner::sign("correct-token", "12345");
        assert!(!AuthSigner::verify("wrong-token", "12345", &sig));
    }

    #[test]
    fn test_hmac_rejects_wrong_message() {
        let sig = AuthSigner::sign("token", "12345");
        assert!(!AuthSigner::verify("token", "99999", &sig));
    }

    #[test]
    fn test_hmac_rejects_tampered_signature() {
        let mut sig = AuthSigner::sign("token", "12345");
        // Flip last char
        let last = sig.pop().unwrap();
        sig.push(if last == 'a' { 'b' } else { 'a' });
        assert!(!AuthSigner::verify("token", "12345", &sig));
    }
}
