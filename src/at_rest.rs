//! Encrypted at-rest message history.
//!
//! Each flock / roost channel keeps its own history file under
//! `config/history/<hash>.bin`. The payload is sealed with
//! [`starling::crypto::EpochKey`] (ChaCha20-Poly1305) under a key derived
//! from the user's profile identity secret with the space's join code as HKDF
//! context — so the file is only readable by this profile; the code alone
//! (shared as an invite) is not enough. The join code is bound as associated
//! data, so a ciphertext copied from one space cannot be replayed in another.
//!
//! Files are written with mode 0600 on unix. Nothing is persisted unless a
//! join code is known, so conversations that were never re-joined have no
//! on-disk trace.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use starling::crypto::EpochKey;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::ui::{FlockView, MessageView, RoostView};

const MAGIC: u32 = 0x5354_4C52; // "STLR"
const VERSION: u32 = 1;
const EPOCH: u64 = 0;
/// Maximum messages kept per conversation. Older entries are dropped.
const MAX_MESSAGES: usize = 2000;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Envelope {
    magic: u32,
    version: u32,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct StoredHistory {
    spaces: Vec<StoredSpace>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct StoredSpace {
    code: String,
    messages: Vec<StoredMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct StoredMessage {
    author: String,
    body: String,
    ts: i64,
}

/// Directory that holds per-conversation history files.
pub fn history_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("history")
}

/// Per-conversation key: HKDF from the profile identity secret with the join
/// code as the (public, non-secret) context. Different codes yield independent
/// keys, and the code alone cannot reproduce the key without the identity
/// secret — so the file is only readable by this profile.
fn key_for(identity_secret: &[u8; 32], code: &str) -> anyhow::Result<EpochKey> {
    EpochKey::derive(identity_secret, code.as_bytes(), EPOCH)
}

/// Path for a conversation's history file. The join code is hashed so the
/// on-disk name reveals nothing about the space.
pub fn path_for(dir: &Path, code: &str) -> PathBuf {
    let digest = Sha256::digest(code.as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.bin"))
}

/// Persist every conversation in `flocks` and `roosts` to disk, encrypted.
/// Files for conversations that no longer exist are pruned.
pub fn save_all(
    dir: &Path,
    identity_secret: &[u8; 32],
    flocks: &[FlockView],
    roosts: &[RoostView],
) -> anyhow::Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let mut written: Vec<PathBuf> = Vec::new();
    for flock in flocks {
        if flock.code.is_empty() || flock.messages.is_empty() {
            continue;
        }
        let path = path_for(dir, &flock.code);
        save_one(&path, identity_secret, &flock.code, &flock.messages)?;
        written.push(path);
    }
    for roost in roosts {
        for channel in &roost.channels {
            if channel.code.is_empty() || channel.messages.is_empty() {
                continue;
            }
            let path = path_for(dir, &channel.code);
            save_one(&path, identity_secret, &channel.code, &channel.messages)?;
            written.push(path);
        }
    }
    for entry in fs::read_dir(dir).with_context(|| format!("failed to list {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "bin") && !written.contains(&path) {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn save_one(
    path: &Path,
    identity_secret: &[u8; 32],
    code: &str,
    messages: &[MessageView],
) -> anyhow::Result<()> {
    let stored = StoredHistory {
        spaces: vec![StoredSpace {
            code: code.to_string(),
            messages: messages
                .iter()
                .rev()
                .take(MAX_MESSAGES)
                .rev()
                .map(|m| StoredMessage {
                    author: m.msg.author.clone(),
                    body: m.msg.body.clone(),
                    ts: m.msg.ts,
                })
                .collect(),
        }],
    };
    let plaintext = postcard::to_stdvec(&stored).context("failed to encode history")?;
    let key = key_for(identity_secret, code)?;
    let (nonce, ciphertext) = key
        .seal(&plaintext, code.as_bytes())
        .context("failed to seal history")?;
    let envelope = Envelope {
        magic: MAGIC,
        version: VERSION,
        nonce,
        ciphertext,
    };
    let bytes = postcard::to_stdvec(&envelope).context("failed to encode history envelope")?;
    anyhow::ensure!(
        bytes.len() <= MAX_FILE_BYTES,
        "history file exceeds size limit"
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{stem}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        file.sync_all().context("failed to fsync history file")?;
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to publish history at {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Load a conversation's history into `view.messages`, replacing any
/// placeholder content. Returns `Ok(false)` when there is no history file.
pub fn load_into_view(
    dir: &Path,
    identity_secret: &[u8; 32],
    code: &str,
    view: &mut FlockView,
) -> anyhow::Result<bool> {
    let Some(plaintext) = load_plaintext(dir, identity_secret, code)? else {
        return Ok(false);
    };
    let stored: StoredHistory =
        postcard::from_bytes(&plaintext).context("failed to decode history")?;
    let Some(space) = stored.spaces.iter().find(|s| s.code == code) else {
        return Ok(false);
    };
    view.messages = space
        .messages
        .iter()
        .map(|m| MessageView {
            msg: starling::event::ChatMessage {
                id: synthetic_id(code, m),
                author: m.author.clone(),
                body: m.body.clone(),
                ts: m.ts,
            },
            private: false,
        })
        .collect();
    Ok(true)
}

fn load_plaintext(
    dir: &Path,
    identity_secret: &[u8; 32],
    code: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let path = path_for(dir, code);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    anyhow::ensure!(bytes.len() <= MAX_FILE_BYTES, "history file is too large");
    let envelope: Envelope =
        postcard::from_bytes(&bytes).context("failed to decode history envelope")?;
    anyhow::ensure!(envelope.magic == MAGIC, "not a history file");
    anyhow::ensure!(envelope.version == VERSION, "unsupported history version");
    let key = key_for(identity_secret, code)?;
    key.open(&envelope.nonce, &envelope.ciphertext, code.as_bytes())
        .context("history decryption failed (key mismatch or tampered file)")
        .map(Some)
}

/// Deterministic id for a persisted message, so re-loads are stable and the
/// live path's uuid ids never collide with them.
fn synthetic_id(code: &str, message: &StoredMessage) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hasher.update(b"\0");
    hasher.update(message.author.as_bytes());
    hasher.update(b"\0");
    hasher.update(message.body.as_bytes());
    hasher.update(b"\0");
    hasher.update(message.ts.to_be_bytes());
    let digest = hasher.finalize();
    let mut id = String::with_capacity(2 + digest.len() * 2);
    id.push_str("h-");
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(id, "{byte:02x}");
    }
    id
}

/// Drop a conversation's history file (used when leaving or deleting).
pub fn remove(dir: &Path, code: &str) -> anyhow::Result<()> {
    let path = path_for(dir, code);
    if path.is_file() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flock(code: &str, messages: Vec<(&str, &str, i64)>) -> FlockView {
        FlockView {
            code: code.to_string(),
            name: code.to_string(),
            messages: messages
                .into_iter()
                .map(|(author, body, ts)| MessageView {
                    msg: starling::event::ChatMessage {
                        id: Uuid::new_v4().to_string(),
                        author: author.to_string(),
                        body: body.to_string(),
                        ts,
                    },
                    private: false,
                })
                .collect(),
            unread: 0,
        }
    }

    #[test]
    fn round_trips_through_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        let flocks = vec![flock(
            "ABC123",
            vec![("Ramhaurg", "hello world", 1), ("Wren", "hi", 2)],
        )];
        save_all(dir.path(), &key, &flocks, &[]).unwrap();
        let mut view = FlockView::default();
        load_into_view(dir.path(), &key, "ABC123", &mut view).unwrap();
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[0].msg.author, "Ramhaurg");
        assert_eq!(view.messages[0].msg.body, "hello world");
        assert_eq!(view.messages[1].msg.body, "hi");
        assert!(view.messages[0].msg.id.starts_with("h-"));
    }

    #[test]
    fn wrong_identity_key_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let flocks = vec![flock("ABC123", vec![("Ramhaurg", "secret text", 1)])];
        save_all(dir.path(), &[1u8; 32], &flocks, &[]).unwrap();
        let mut view = FlockView::default();
        assert!(
            load_into_view(dir.path(), &[2u8; 32], "ABC123", &mut view).is_err(),
            "decrypting with a different profile key must fail"
        );
        assert!(view.messages.is_empty());
    }

    #[test]
    fn tampered_file_fails_authentication() {
        let dir = tempfile::tempdir().unwrap();
        let key = [3u8; 32];
        let flocks = vec![flock("ABC123", vec![("Ramhaurg", "integrity matters", 1)])];
        save_all(dir.path(), &key, &flocks, &[]).unwrap();
        let path = path_for(dir.path(), "ABC123");
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        fs::write(&path, &bytes).unwrap();
        let mut view = FlockView::default();
        assert!(load_into_view(dir.path(), &key, "ABC123", &mut view).is_err());
    }

    #[test]
    fn codes_are_isolated_and_files_are_0600() {
        let dir = tempfile::tempdir().unwrap();
        let key = [9u8; 32];
        let flocks = vec![
            flock("CODE-A", vec![("A", "alpha", 1)]),
            flock("CODE-B", vec![("B", "beta", 2)]),
        ];
        save_all(dir.path(), &key, &flocks, &[]).unwrap();
        let mut a = FlockView::default();
        let mut b = FlockView::default();
        load_into_view(dir.path(), &key, "CODE-A", &mut a).unwrap();
        load_into_view(dir.path(), &key, "CODE-B", &mut b).unwrap();
        assert_eq!(a.messages.len(), 1);
        assert_eq!(a.messages[0].msg.body, "alpha");
        assert_eq!(b.messages[0].msg.body, "beta");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path_for(dir.path(), "CODE-A"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn leaving_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let key = [5u8; 32];
        let flocks = vec![flock("CODE-X", vec![("A", "bye", 1)])];
        save_all(dir.path(), &key, &flocks, &[]).unwrap();
        assert!(path_for(dir.path(), "CODE-X").is_file());
        remove(dir.path(), "CODE-X").unwrap();
        assert!(!path_for(dir.path(), "CODE-X").is_file());
    }

    #[test]
    fn save_all_prunes_orphaned_files() {
        let dir = tempfile::tempdir().unwrap();
        let key = [6u8; 32];
        let flocks = vec![flock("KEEP", vec![("A", "stay", 1)])];
        save_all(dir.path(), &key, &flocks, &[]).unwrap();
        // A conversation that is gone from the views gets cleaned up.
        save_all(dir.path(), &key, &[], &[]).unwrap();
        assert!(!path_for(dir.path(), "KEEP").is_file());
    }
}
