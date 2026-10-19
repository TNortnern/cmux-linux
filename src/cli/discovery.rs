//! Socket path discovery chain for the cmux CLI.
//!
//! Ports the Python `_default_socket_path()` logic from tests_v2/cmux.py to Rust.
//! Discovery order matches D-11, minus the world-writable /tmp fallbacks: another
//! local user could plant a socket there and receive our commands.

use std::os::unix::fs::{FileTypeExt, MetadataExt};

/// Discover the cmux socket path using the standard search chain.
///
/// Returns `Some(path)` if a candidate is found, `None` otherwise.
/// Explicit overrides (CMUX_SOCKET, CMUX_SOCKET_PATH, --socket) are returned as given.
/// Discovered paths must pass `is_trusted_socket` (own socket, not a symlink).
pub fn discover_socket() -> Option<String> {
    // 1. CMUX_SOCKET env var
    if let Ok(val) = std::env::var("CMUX_SOCKET") {
        if !val.is_empty() {
            return Some(val);
        }
    }

    // Also check CMUX_SOCKET_PATH for backwards compat with Python client
    if let Ok(val) = std::env::var("CMUX_SOCKET_PATH") {
        if !val.is_empty() {
            return Some(val);
        }
    }

    // 2. $XDG_RUNTIME_DIR/cmux/cmux.sock (fallback /run/user/{uid}/cmux/cmux.sock)
    let xdg_base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        format!("/run/user/{}", unsafe { libc::getuid() })
    });
    let xdg_socket = format!("{}/cmux/cmux.sock", xdg_base);
    if trusted_socket_path(&xdg_socket) {
        return Some(xdg_socket);
    }

    // 3. $XDG_RUNTIME_DIR/cmux/last-socket-path marker file
    let marker = format!("{}/cmux/last-socket-path", xdg_base);
    if let Ok(contents) = std::fs::read_to_string(&marker) {
        let path = contents.trim().to_string();
        if !path.is_empty() && trusted_socket_path(&path) {
            return Some(path);
        }
    }

    None
}

/// Pure ownership rule for a discovered socket, from its lstat fields.
/// Must be a socket (not a regular file or directory), not a symlink, owned by `expected_uid`.
pub fn is_trusted_socket(is_socket: bool, is_symlink: bool, owner_uid: u32, expected_uid: u32) -> bool {
    is_socket && !is_symlink && owner_uid == expected_uid
}

/// lstat `path` (no symlink following) and apply `is_trusted_socket` for our own UID.
fn trusted_socket_path(path: &str) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let file_type = meta.file_type();
            is_trusted_socket(
                file_type.is_socket(),
                file_type.is_symlink(),
                meta.uid(),
                unsafe { libc::getuid() },
            )
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_socket_is_trusted() {
        assert!(is_trusted_socket(true, false, 1000, 1000));
    }

    #[test]
    fn foreign_owner_is_rejected() {
        assert!(!is_trusted_socket(true, false, 1001, 1000));
        assert!(!is_trusted_socket(true, false, 0, 1000));
    }

    #[test]
    fn symlink_is_rejected() {
        assert!(!is_trusted_socket(true, true, 1000, 1000));
    }

    #[test]
    fn non_socket_is_rejected() {
        assert!(!is_trusted_socket(false, false, 1000, 1000));
    }
}
