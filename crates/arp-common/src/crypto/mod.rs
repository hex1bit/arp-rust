use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

use crate::error::{Error, Result};

pub struct Compressor;
pub struct PacketCipher;

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

fn derive_key(secret: &str) -> [u8; 32] {
    let digest = Sha256::digest(secret.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    out
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
    fn test_decrypt_with_wrong_secret() {
        let original = b"hello secure world";
        let encrypted = PacketCipher::encrypt(original, "secret-a").unwrap();
        assert!(PacketCipher::decrypt(&encrypted, "secret-b").is_err());
    }
}
