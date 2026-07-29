//! Locate or download the target-specific static Opus library and bindings.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const OPUS_VERSION: &str = "2026.1.0";

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_AUDIO");
    if env::var_os("CARGO_FEATURE_AUDIO").is_none() {
        return;
    }

    println!("cargo:rerun-if-env-changed=LIBOPUS_LIB_DIR");
    println!("cargo:rerun-if-env-changed=LIBOPUS_BINDINGS_PATH");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let lib_dir = out_dir.join("opus_lib");
    let bindings_path = out_dir.join("opus_bindings.rs");

    if let Some(system_lib_dir) = env::var_os("LIBOPUS_LIB_DIR") {
        let supplied_bindings = env::var_os("LIBOPUS_BINDINGS_PATH")
            .expect("LIBOPUS_BINDINGS_PATH must be set when LIBOPUS_LIB_DIR is used");
        fs::copy(supplied_bindings, &bindings_path).expect("failed to copy LIBOPUS_BINDINGS_PATH");
        link_opus(Path::new(&system_lib_dir));
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("target OS not set by Cargo");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("target arch not set by Cargo");

    // shiguredo does not publish an Intel macOS archive. Use the native Opus
    // installed by Homebrew (or another system package manager) instead.
    if target_os == "macos" && target_arch == "x86_64" {
        write_system_bindings(&bindings_path);
        let opus_prefix = Command::new("brew")
            .args(["--prefix", "opus"])
            .output()
            .expect("Homebrew is required to locate Opus on Intel macOS");
        if !opus_prefix.status.success() {
            panic!("Homebrew Opus is not installed; run `brew install opus`");
        }
        let prefix =
            String::from_utf8(opus_prefix.stdout).expect("Homebrew returned a non-UTF-8 Opus path");
        let lib_dir = PathBuf::from(prefix.trim()).join("lib");
        link_opus(&lib_dir);
        return;
    }

    if lib_dir.exists() && bindings_path.exists() {
        link_opus(&lib_dir);
        return;
    }
    let asset_target = match (target_os.as_str(), target_arch.as_str()) {
        ("windows", "x86_64") => "windows_x86_64",
        ("linux", "x86_64") => "ubuntu-24.04_x86_64",
        ("linux", "aarch64") => "ubuntu-24.04_arm64",
        ("macos", "aarch64") => "macos_arm64",
        _ => panic!(
            "unsupported Opus target {target_arch}-{target_os}; set LIBOPUS_LIB_DIR and LIBOPUS_BINDINGS_PATH"
        ),
    };
    let lib_file = if target_os == "windows" {
        "opus.lib"
    } else {
        "libopus.a"
    };
    let url = format!(
        "https://github.com/shiguredo/opus-rs/releases/download/{OPUS_VERSION}/libopus-{asset_target}.tar.gz"
    );

    let archive_path = out_dir.join("libopus.tar.gz");
    let partial_path = out_dir.join("libopus.tar.gz.part");
    eprintln!("Downloading pre-built Opus: {url}");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--retry",
            "3",
            "--output",
        ])
        .arg(&partial_path)
        .arg(&url)
        .status()
        .expect(
            "failed to run curl; install curl or set LIBOPUS_LIB_DIR and LIBOPUS_BINDINGS_PATH",
        );
    if !status.success() {
        let _ = fs::remove_file(&partial_path);
        panic!("failed to download pre-built Opus from {url}");
    }
    fs::rename(&partial_path, &archive_path).expect("failed to finalize Opus download");

    validate_archive(&archive_path);
    let extract_dir = out_dir.join("opus_extract");
    let _ = fs::remove_dir_all(&extract_dir);
    fs::create_dir_all(&extract_dir).expect("failed to create Opus extraction directory");
    let status = Command::new("tar")
        .arg("xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract_dir)
        .status()
        .expect("failed to run tar; install tar or set LIBOPUS_LIB_DIR and LIBOPUS_BINDINGS_PATH");
    if !status.success() {
        panic!("failed to extract Opus archive");
    }

    fs::create_dir_all(&lib_dir).expect("failed to create Opus library directory");
    let source_library = extract_dir.join("lib").join(lib_file);
    fs::copy(&source_library, lib_dir.join(lib_file)).unwrap_or_else(|error| {
        panic!(
            "failed to copy {} from the Opus archive: {error}",
            source_library.display()
        )
    });
    fs::copy(extract_dir.join("bindings.rs"), &bindings_path)
        .expect("failed to copy Opus bindings from archive");

    link_opus(&lib_dir);
}

fn write_system_bindings(path: &Path) {
    const BINDINGS: &str = r#"
pub const OPUS_APPLICATION_VOIP: u32 = 2048;

#[repr(C)]
pub struct OpusEncoder {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OpusDecoder {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub fn opus_encoder_create(
        fs: ::std::os::raw::c_int,
        channels: ::std::os::raw::c_int,
        application: ::std::os::raw::c_int,
        error: *mut ::std::os::raw::c_int,
    ) -> *mut OpusEncoder;
    pub fn opus_encode_float(
        st: *mut OpusEncoder,
        pcm: *const f32,
        frame_size: ::std::os::raw::c_int,
        data: *mut u8,
        max_data_bytes: ::std::os::raw::c_int,
    ) -> ::std::os::raw::c_int;
    pub fn opus_encoder_destroy(st: *mut OpusEncoder);
    pub fn opus_decoder_create(
        fs: ::std::os::raw::c_int,
        channels: ::std::os::raw::c_int,
        error: *mut ::std::os::raw::c_int,
    ) -> *mut OpusDecoder;
    pub fn opus_decode_float(
        st: *mut OpusDecoder,
        data: *const u8,
        len: ::std::os::raw::c_int,
        pcm: *mut f32,
        frame_size: ::std::os::raw::c_int,
        decode_fec: ::std::os::raw::c_int,
    ) -> ::std::os::raw::c_int;
    pub fn opus_decoder_destroy(st: *mut OpusDecoder);
}
"#;
    fs::write(path, BINDINGS).expect("failed to write system Opus bindings");
}

fn validate_archive(archive_path: &Path) {
    let output = Command::new("tar")
        .arg("tzf")
        .arg(archive_path)
        .output()
        .expect("failed to inspect Opus archive");
    if !output.status.success() {
        panic!("downloaded Opus archive is invalid");
    }
    let listing = String::from_utf8(output.stdout).expect("Opus archive contains invalid paths");
    for entry in listing.lines() {
        let path = Path::new(entry);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            panic!("unsafe path in Opus archive: {entry}");
        }
    }
}

fn link_opus(lib_dir: &Path) {
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=opus");
}
