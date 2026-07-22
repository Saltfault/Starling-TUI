//! Simple file logger. On startup, the previous `latest.log` is gzipped and
//! archived with a timestamp. A fresh `latest.log` is created for the new
//! session.
//!
//! Log layout:
//! ```text
//! logs/
//!   latest.log                   ← current session
//!   2025-07-21_14-32-05.log.gz   ← previous session, gzipped
//!   2025-07-21_13-01-22.log.gz
//! ```
//!
//! All functions are thread-safe (the log file is opened in append mode for
//! each write, which is atomic on Unix).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use flate2::Compression;
use flate2::write::GzEncoder;

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Initialize the logger. Call this at the very start of `main()`, before
/// anything else.
///
/// 1. Creates the `logs/` directory if it doesn't exist.
/// 2. If `logs/latest.log` exists from a previous session, gzips it to
///    `logs/<timestamp>.log.gz` and removes the original.
/// 3. Creates a fresh `latest.log` with a startup banner.
pub fn init() {
    let log_dir = PathBuf::from("logs");
    fs::create_dir_all(&log_dir).ok();

    let latest = log_dir.join("latest.log");

    // Archive the previous session's log.
    if latest.exists() {
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let gz_path = log_dir.join(format!("{timestamp}.log.gz"));

        if let Ok(data) = fs::read(&latest) {
            if let Ok(gz_file) = File::create(&gz_path) {
                let mut encoder = GzEncoder::new(gz_file, Compression::default());
                let _ = encoder.write_all(&data);
                let _ = encoder.finish();
            }
        }

        let _ = fs::remove_file(&latest);
    }

    // Store the log directory for later writes.
    let _ = LOG_DIR.set(log_dir.clone());

    // Write a startup banner to the new log.
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = fs::write(
        log_dir.join("latest.log"),
        format!("[{timestamp}] === Starling started ===\n"),
    );
}

/// Write a line to `latest.log`. Silently does nothing if [`init`] wasn't
/// called. `level` includes trailing spacing (e.g. `"ERROR: "`) so messages
/// align across levels.
fn log(level: &str, msg: &str) {
    let Some(dir) = LOG_DIR.get() else { return };
    let path = dir.join("latest.log");

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{timestamp}] {level}{msg}\n");

    if let Ok(mut file) = OpenOptions::new().append(true).create(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Log an error message with a timestamp.
#[allow(dead_code)]
pub fn error(msg: &str) {
    log("ERROR: ", msg);
}

/// Log a warning message with a timestamp.
pub fn warn(msg: &str) {
    log("WARN:  ", msg);
}
