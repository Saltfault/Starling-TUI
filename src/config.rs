//! User profile persistence: display name and audio device preferences,
//! saved to disk as a postcard-serialized binary file.
//!
//! - Linux/macOS: `~/.config/starling/profile.bin`
//! - Windows:     `%APPDATA%\starling\profile.bin`
//!
//! The display name can also be shared between machines as a compact
//! 32-hex-digit code (see [`Profile::to_code`]). Device settings are
//! machine-specific and don't transfer.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User settings persisted across sessions.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Profile {
    /// Display name shown next to messages.
    pub name: String,
    /// Preferred input device name (from cpal). `None` = system default.
    pub input_device: Option<String>,
    /// Preferred output device name (from cpal). `None` = system default.
    pub output_device: Option<String>,
}

impl Profile {
    /// Return the platform-appropriate config directory.
    pub fn config_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("starling")
        } else if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("starling")
        } else {
            PathBuf::from(".starling")
        }
    }

    /// Return the directory where roost data directories live.
    #[allow(dead_code)]
    pub fn roosts_dir() -> PathBuf {
        Self::config_dir().join("roosts")
    }

    /// Path to the profile file on disk.
    fn config_path() -> PathBuf {
        Self::config_dir().join("profile.bin")
    }

    /// Load the profile from disk. Returns `None` if no profile exists.
    pub fn load() -> Option<Self> {
        let data = std::fs::read(Self::config_path()).ok()?;
        postcard::from_bytes(&data).ok()
    }

    /// Save the profile to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)?;
        let data = postcard::to_stdvec(self)?;
        std::fs::write(Self::config_path(), data)?;
        Ok(())
    }

    /// Encode the profile name as a 32-hex-digit code.
    ///
    /// Format: 1 byte length + up to 15 bytes name = 16 bytes = 32 hex digits.
    pub fn to_code(&self) -> String {
        let name_bytes = self.name.as_bytes();
        let len = name_bytes.len().min(15) as u8;
        let mut buf = [0u8; 16];
        buf[0] = len;
        buf[1..1 + len as usize].copy_from_slice(&name_bytes[..len as usize]);
        data_encoding::HEXUPPER.encode(&buf)
    }

    /// Decode a 32-hex-digit code into a profile (name only).
    pub fn from_code(code: &str) -> Option<Self> {
        let bytes = data_encoding::HEXUPPER.decode(code.as_bytes()).ok()?;
        if bytes.len() != 16 {
            return None;
        }
        let len = bytes[0] as usize;
        if len > 15 {
            return None;
        }
        let name = String::from_utf8(bytes[1..1 + len].to_vec()).ok()?;
        Some(Profile {
            name,
            input_device: None,
            output_device: None,
        })
    }

    /// Load the persistent identity key, creating one on the first run.
    pub fn load_or_create_secret() -> iroh::SecretKey {
        let path = Self::config_dir().join("identity.key");
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return iroh::SecretKey::from_bytes(&arr);
            }
        }

        let key = iroh::SecretKey::generate();
        let _ = std::fs::create_dir_all(Self::config_dir());
        let _ = std::fs::write(&path, key.to_bytes());
        key
    }
}
