//! Build script: downloads a pre-built opus static library from the
//! shiguredo/opus-rs GitHub releases. No CMake required — just curl + tar.
//!
//! Supported platforms:
//! - Windows x86_64  → opus.lib
//! - Linux x86_64     → libopus.a  (Ubuntu 24.04 build, works on any x86_64)
//! - Linux aarch64    → libopus.a
//! - macOS aarch64    → libopus.a

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let lib_dir = out_dir.join("opus_lib");
    let bindings_path = out_dir.join("opus_bindings.rs");

    // Skip if already downloaded (incremental build).
    if lib_dir.exists() && bindings_path.exists() {
        link_opus(&lib_dir);
        return;
    }

    // Determine the platform target string.
    let target = if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows_x86_64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "ubuntu-24.04_x86_64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "ubuntu-24.04_arm64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos_arm64"
    } else {
        panic!(
            "unsupported platform for pre-built opus. Set LIBOPUS_LIB_DIR to a pre-built opus library."
        );
    };

    let version = "2026.1.0";
    let url = format!(
        "https://github.com/shiguredo/opus-rs/releases/download/{}/libopus-{}.tar.gz",
        version, target
    );

    // Download.
    let archive_path = out_dir.join("libopus.tar.gz");
    eprintln!("Downloading pre-built opus: {url}");
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive_path)
        .arg(&url)
        .status()
        .expect("failed to run curl. Ensure curl is installed.");
    if !status.success() {
        panic!("failed to download pre-built opus: {url}");
    }

    // Extract.
    let extract_dir = out_dir.join("opus_extract");
    fs::create_dir_all(&extract_dir).expect("failed to create extract dir");
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract_dir)
        .status()
        .expect("failed to run tar. Ensure tar is installed.");
    if !status.success() {
        panic!("failed to extract opus archive");
    }

    // Copy library file (opus.lib on Windows, libopus.a on Unix).
    fs::create_dir_all(&lib_dir).expect("failed to create lib dir");
    let lib_file = if cfg!(target_os = "windows") {
        "opus.lib"
    } else {
        "libopus.a"
    };
    let src_lib = extract_dir.join("lib").join(lib_file);
    fs::copy(&src_lib, lib_dir.join(lib_file))
        .unwrap_or_else(|_| panic!("failed to copy {lib_file} from extracted archive"));

    // Copy bindings.rs.
    fs::copy(extract_dir.join("bindings.rs"), &bindings_path).expect("failed to copy bindings.rs");

    link_opus(&lib_dir);
}

fn link_opus(lib_dir: &std::path::Path) {
    println!("cargo:rustc-link-search={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=opus");
}
