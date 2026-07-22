//! Small platform utilities.
//!
//! On Unix, [`suppress_stderr`] temporarily redirects fd 2 to `/dev/null`
//! so that C libraries (like ALSA) can't spam the terminal with error
//! messages when they fail to find audio hardware.

/// Run `f` with stderr silenced. Any error messages written to fd 2 by
/// native code (e.g. ALSA's "cannot find card '0'") are discarded.
///
/// On non-Unix platforms this is a no-op.
#[allow(dead_code)]
#[cfg(unix)]
pub fn suppress_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use std::os::unix::io::RawFd;

    const STDERR: RawFd = 2;

    unsafe {
        // Save the original stderr fd and open /dev/null.
        let saved = libc::dup(STDERR);
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY);

        if devnull >= 0 && saved >= 0 {
            libc::dup2(devnull, STDERR);
        }

        let result = f();

        // Restore stderr.
        if saved >= 0 {
            libc::dup2(saved, STDERR);
            libc::close(saved);
        }
        if devnull >= 0 {
            libc::close(devnull);
        }

        result
    }
}

#[allow(dead_code)]
#[cfg(not(unix))]
pub fn suppress_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}
