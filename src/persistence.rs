use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use starling::protocol::SpaceId;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;
use zeroize::Zeroize;

const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextDescriptor {
    pub space: SpaceId,
    pub label: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicState {
    /// Display order is significant and is preserved during serialization.
    pub contexts: Vec<ContextDescriptor>,
    pub active_space: Option<SpaceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct Credential {
    pub name: String,
    pub secret: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct ProtectedSecretState {
    pub credentials: Vec<Credential>,
}

pub fn load_public(path: impl AsRef<Path>) -> anyhow::Result<PublicState> {
    load(path)
}

pub fn save_public(path: impl AsRef<Path>, state: &PublicState) -> anyhow::Result<()> {
    save(path.as_ref(), state, false)
}

pub fn load_protected(path: impl AsRef<Path>) -> anyhow::Result<ProtectedSecretState> {
    load(path)
}

pub fn save_protected(path: impl AsRef<Path>, state: &ProtectedSecretState) -> anyhow::Result<()> {
    save(path.as_ref(), state, true)
}

pub fn recover(path: impl AsRef<Path>) -> anyhow::Result<()> {
    let path = path.as_ref();
    let _lock = WriterLock::acquire(path)?;
    recover_locked(path)
}

fn load<T: DeserializeOwned>(path: impl AsRef<Path>) -> anyhow::Result<T> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect state at {}", path.display()))?;
    ensure!(metadata.is_file(), "state path is not a regular file");
    ensure!(
        metadata.len() <= MAX_STATE_BYTES as u64,
        "state file is too large"
    );
    let bytes =
        fs::read(path).with_context(|| format!("failed to read state at {}", path.display()))?;
    ensure!(
        bytes.len() <= MAX_STATE_BYTES,
        "state file grew while being read"
    );
    postcard::from_bytes(&bytes).context("failed to decode persisted state")
}

fn save<T: Serialize>(path: &Path, value: &T, secret: bool) -> anyhow::Result<()> {
    let bytes = postcard::to_stdvec(value).context("failed to encode persisted state")?;
    ensure!(bytes.len() <= MAX_STATE_BYTES, "encoded state is too large");
    let parent = parent(path)?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create state directory {}", parent.display()))?;
    let _lock = WriterLock::acquire(path)?;
    recover_locked(path)?;
    replace_locked(path, &bytes, secret)
}

fn replace_locked(path: &Path, bytes: &[u8], secret: bool) -> anyhow::Result<()> {
    let parent = parent(path)?;
    let stem = file_name(path)?;
    let temporary = parent.join(format!(".{stem}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = open_new(&temporary, secret)?;
        file.write_all(bytes)
            .context("failed to write temporary state")?;
        file.sync_all().context("failed to fsync temporary state")?;
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to atomically publish state at {}", path.display()))?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn recover_locked(path: &Path) -> anyhow::Result<()> {
    let parent = parent(path)?;
    if !parent.exists() {
        return Ok(());
    }
    let stem = file_name(path)?;
    let tmp_prefix = format!(".{stem}.");
    let mut backups = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&tmp_prefix) && name.ends_with(".tmp") {
            if entry.file_type()?.is_file() {
                fs::remove_file(entry.path())?;
            }
        } else if name.starts_with(&tmp_prefix)
            && name.ends_with(".bak")
            && entry.file_type()?.is_file()
        {
            backups.push(entry.path());
        }
    }
    backups.sort();
    if !path.exists()
        && let Some(recovery) = backups.pop()
    {
        fs::rename(&recovery, path).with_context(|| "failed to recover persisted state")?;
    }
    for backup in backups {
        fs::remove_file(backup)?;
    }
    sync_directory(parent)
}

fn open_new(path: &Path, secret: bool) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    #[cfg(windows)]
    if secret {
        // Starling's contract requires a current-user-only parent ACL on Windows.
        // Refuse to imply that a regular file grants keyring-equivalent protection.
        let _ = secret;
    }
    Ok(file)
}

fn sync_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(path)?
        .sync_all()
        .context("failed to fsync state directory")?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn parent(path: &Path) -> anyhow::Result<&Path> {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .context("state path has no parent directory")
}

fn file_name(path: &Path) -> anyhow::Result<String> {
    Ok(path
        .file_name()
        .context("state path has no file name")?
        .to_string_lossy()
        .into_owned())
}

struct WriterLock(PathBuf);

impl WriterLock {
    fn acquire(path: &Path) -> anyhow::Result<Self> {
        let parent = parent(path)?;
        fs::create_dir_all(parent)?;
        let lock = parent.join(format!(".{}.lock", file_name(path)?));
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(mut file) => {
                    writeln!(file, "{}", Uuid::new_v4())?;
                    file.sync_all()?;
                    return Ok(Self(lock));
                }
                Err(error) if lock_contention(&error) && started.elapsed() < LOCK_TIMEOUT => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) if lock_contention(&error) => {
                    return Err(error).context("timed out waiting for persisted-state writer lock");
                }
                Err(error) => {
                    return Err(error).context("failed to acquire persisted-state writer lock");
                }
            }
        }
    }
}

fn lock_contention(error: &std::io::Error) -> bool {
    // Windows races can surface create-new contention as PermissionDenied or
    // raw ERROR_FILE_EXISTS (80), in addition to AlreadyExists.
    matches!(
        error.kind(),
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
    ) || error.raw_os_error() == Some(80)
        || error.raw_os_error() == Some(17) // EEXIST on Unix
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starling::protocol::FlockId;
    use std::sync::{Arc, Barrier};

    fn state(value: u8) -> PublicState {
        PublicState {
            contexts: vec![ContextDescriptor {
                space: SpaceId::Flock(FlockId([value; 32])),
                label: format!("context-{value}"),
            }],
            active_space: Some(SpaceId::Flock(FlockId([value; 32]))),
        }
    }

    #[test]
    fn exit_save_preserves_existing_credentials() {
        // The TUI main() keeps the loaded ProtectedSecretState and writes it
        // back at exit. This test ensures credentials are not silently wiped.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.bin");
        let credentials = ProtectedSecretState {
            credentials: vec![Credential {
                name: "roost-key".into(),
                secret: vec![1; 32],
            }],
        };
        save_protected(&path, &credentials).unwrap();
        assert!(!load_protected(&path).unwrap().credentials.is_empty());

        // Simulating the exit-path in Starling-TUI/src/main.rs: the same state
        // that was loaded is saved back.
        save_protected(&path, &credentials).unwrap();
        assert!(!load_protected(&path).unwrap().credentials.is_empty());
    }

    #[test]
    fn saves_loads_and_overwrites_without_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        save_public(&path, &state(1)).unwrap();
        save_public(&path, &state(2)).unwrap();
        assert_eq!(load_public(&path).unwrap(), state(2));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn cleans_temps_and_recovers_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        let backup = dir.path().join(".state.bin.old.bak");
        let temporary = dir.path().join(".state.bin.old.tmp");
        fs::write(&backup, postcard::to_stdvec(&state(3)).unwrap()).unwrap();
        fs::write(&temporary, b"partial").unwrap();
        recover(&path).unwrap();
        assert_eq!(load_public(&path).unwrap(), state(3));
        assert!(!temporary.exists());
        assert!(!backup.exists());
    }

    #[test]
    fn concurrent_writers_always_publish_complete_serializations() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("state.bin"));
        let barrier = Arc::new(Barrier::new(5));
        let mut threads = Vec::new();
        for value in 0..4 {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..10 {
                    save_public(path.as_ref(), &state(value)).unwrap();
                }
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        let loaded = load_public(path.as_ref()).unwrap();
        assert!((0..4).any(|value| loaded == state(value)));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn protected_state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.bin");
        let secret = ProtectedSecretState {
            credentials: vec![Credential {
                name: "token".into(),
                secret: vec![7; 32],
            }],
        };
        save_protected(&path, &secret).unwrap();
        assert_eq!(load_protected(&path).unwrap(), secret);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
