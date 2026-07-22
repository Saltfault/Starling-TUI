//! End-to-end encryption for gossip messages.
//!
//! A symmetric key is derived from the room code via SHA-256. Every text
//! message is encrypted with ChaCha20-Poly1305 before being broadcast over
//! gossip, and decrypted on receipt. Anyone without the room code (including
//! relays) cannot read the message content.
//!
//! Voice calls are already E2E encrypted via iroh's QUIC TLS 1.3 — this
//! module only handles gossip text messages.
//!
//! Encrypted message format: `[12-byte nonce | ciphertext + 16-byte tag]`.

use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, aead::Aead};
use sha2::{Digest, Sha256};

/// Encryption context for a flock. Derived from the room code.
pub struct FlockCrypto {
    cipher: ChaCha20Poly1305,
}

impl FlockCrypto {
    /// Create a crypto context from a room code. All birds in the same flock
    /// derive the same key.
    pub fn from_room_code(code: &str) -> Self {
        let key = Sha256::digest(format!("starling/flock/{code}").as_bytes());
        let cipher = ChaCha20Poly1305::new(&key);
        Self { cipher }
    }

    /// Encrypt plaintext. Returns `[nonce | ciphertext]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let uuid = uuid::Uuid::new_v4();
        let nonce_bytes: [u8; 12] = uuid.as_bytes()[..12].try_into().unwrap();
        let nonce = Nonce::from(nonce_bytes);

        let ciphertext = self.cipher.encrypt(&nonce, plaintext).unwrap_or_default();

        let mut output = nonce_bytes.to_vec();
        output.extend(ciphertext);
        output
    }

    /// Decrypt `[nonce | ciphertext]`. Returns `None` on failure.
    pub fn decrypt(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < 12 {
            return None;
        }
        let nonce_bytes: [u8; 12] = data[..12].try_into().ok()?;
        let nonce = Nonce::from(nonce_bytes);
        self.cipher.decrypt(&nonce, &data[12..]).ok()
    }
}
