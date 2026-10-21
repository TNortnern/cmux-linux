//! Working-directory tracking for a pane's shell.
//!
//! A shell with Ghostty shell integration reports its folder through the PWD action
//! (OSC 7); that is the primary source. A shell without integration is found through /proc:
//! every shell is started with `CMUX_PANE_ID=<pane id>` in its environment, so its process
//! can be located among cmux's children, and its folder is read from `/proc/<pid>/cwd`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Environment variable that tags a pane's shell with its pane id.
pub const PANE_ID_ENV: &str = "CMUX_PANE_ID";

/// What cmux knows about one pane's shell. Shared between the pane's context and the Ghostty
/// action callback, which can run off the main thread, so it lives behind a Mutex.
#[derive(Debug, Default)]
pub struct PaneTrack {
    /// Last absolute folder the shell reported.
    pub cwd: Option<String>,
    /// Shell pid found through /proc, cached. Re-checked before each use.
    pub shell_pid: Option<i32>,
    /// SSH pane: its shell runs remotely, so its folders are not local paths and are not tracked.
    pub remote: bool,
    /// Folder the pane was started in. Only a fallback: it answers until the shell reports a
    /// folder or /proc finds the shell, so it is never preferred over the shell's own report.
    pub initial: Option<PathBuf>,
}

/// Shared handle to a pane's `PaneTrack`.
pub type PaneTrackCell = Arc<Mutex<PaneTrack>>;

/// A fresh, empty track for a new pane.
pub fn new_track() -> PaneTrackCell {
    Arc::new(Mutex::new(PaneTrack::default()))
}

/// Accepts a folder only if it is an absolute path to an existing directory.
pub fn validate_cwd(path: &Path) -> Option<PathBuf> {
    (path.is_absolute() && path.is_dir()).then(|| path.to_path_buf())
}

/// True if a NUL-separated environment block (the format of /proc/<pid>/environ) contains
/// `CMUX_PANE_ID=<pane_id>` as a whole entry.
pub fn environ_has_pane_id(environ: &[u8], pane_id: u64) -> bool {
    let wanted = format!("{PANE_ID_ENV}={pane_id}");
    environ
        .split(|&b| b == 0)
        .any(|entry| entry == wanted.as_bytes())
}

/// Parent pid from the contents of /proc/<pid>/stat ("pid (comm) state ppid ...").
/// `comm` may contain spaces and parentheses, so the fields are read after the last ')'.
fn ppid_from_stat(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn pid_has_pane_id(pid: i32, pane_id: u64) -> bool {
    std::fs::read(format!("/proc/{pid}/environ"))
        .map(|environ| environ_has_pane_id(&environ, pane_id))
        .unwrap_or(false)
}

/// Finds the shell of `pane_id`: a direct child of this process whose environment carries
/// `CMUX_PANE_ID=<pane_id>`. Scans /proc, so callers should cache the result.
pub fn shell_pid_for_pane(pane_id: u64) -> Option<i32> {
    let our_pid = std::process::id();
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        if ppid_from_stat(&stat) != Some(our_pid) {
            continue;
        }
        if pid_has_pane_id(pid as i32, pane_id) {
            return Some(deepest_tagged_descendant(pid as i32, pane_id));
        }
    }
    None
}

/// Ghostty may start the shell through a wrapper (`sh -c ...` → bash). The wrapper never changes
/// folder, so follow the chain down while a process has exactly one child carrying the same tag.
fn deepest_tagged_descendant(mut pid: i32, pane_id: u64) -> i32 {
    for _ in 0..8 {
        let children = children_of(pid);
        match children.as_slice() {
            [only] if pid_has_pane_id(*only, pane_id) => pid = *only,
            _ => break,
        }
    }
    pid
}

fn children_of(pid: i32) -> Vec<i32> {
    // /proc/<pid>/task/<pid>/children lists direct children (Linux 3.5+).
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .map(|s| s.split_whitespace().filter_map(|c| c.parse().ok()).collect())
        .unwrap_or_default()
}

/// Folder of a pane's shell. Uses the folder the shell reported when it still exists; otherwise
/// the /proc cwd of the shell (its pid is cached in `track`). None if neither is known.
pub fn pane_cwd(pane_id: u64, track: &PaneTrackCell) -> Option<PathBuf> {
    let (reported, cached_pid, initial) = {
        let t = track.lock().ok()?;
        if t.remote {
            return None;
        }
        (t.cwd.clone(), t.shell_pid, t.initial.clone())
    };
    if let Some(path) = reported.as_deref().and_then(|s| validate_cwd(Path::new(s))) {
        return Some(path);
    }
    proc_cwd(pane_id, track, cached_pid).or_else(|| initial.and_then(|p| validate_cwd(&p)))
}

/// Folder of the pane's shell as /proc reports it. None until the shell exists.
fn proc_cwd(pane_id: u64, track: &PaneTrackCell, cached_pid: Option<i32>) -> Option<PathBuf> {
    let pid = match cached_pid.filter(|&pid| pid_has_pane_id(pid, pane_id)) {
        Some(pid) => pid,
        None => {
            let found = shell_pid_for_pane(pane_id)?;
            if let Ok(mut t) = track.lock() {
                t.shell_pid = Some(found);
            }
            found
        }
    };
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .and_then(|path| validate_cwd(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environ_finds_whole_pane_id_entry() {
        let environ = b"HOME=/root\0CMUX_PANE_ID=7\0PATH=/bin\0";
        assert!(environ_has_pane_id(environ, 7));
        // Prefixes and longer ids must not match.
        assert!(!environ_has_pane_id(environ, 1));
        assert!(!environ_has_pane_id(b"CMUX_PANE_ID=77\0", 7));
        assert!(!environ_has_pane_id(b"XCMUX_PANE_ID=7\0", 7));
        assert!(!environ_has_pane_id(b"", 7));
    }

    #[test]
    fn stat_ppid_handles_spaces_and_parens_in_comm() {
        assert_eq!(ppid_from_stat("123 (cmux app) S 42 123 123 0"), Some(42));
        assert_eq!(ppid_from_stat("9 (a) b) S 5 9 9 0"), Some(5));
        assert_eq!(ppid_from_stat("garbage"), None);
    }

    #[test]
    fn cwd_validation_accepts_only_existing_absolute_dirs() {
        assert_eq!(validate_cwd(Path::new("/")), Some(PathBuf::from("/")));
        assert_eq!(validate_cwd(Path::new("relative/dir")), None);
        assert_eq!(validate_cwd(Path::new("/definitely/not/here-xyz")), None);
        // A regular file is absolute and exists, but is not a folder.
        assert_eq!(validate_cwd(Path::new("/proc/self/status")), None);
    }

    #[test]
    fn reported_cwd_wins_over_proc() {
        let track = new_track();
        track.lock().unwrap().cwd = Some("/".to_string());
        assert_eq!(pane_cwd(u64::MAX, &track), Some(PathBuf::from("/")));
    }

    /// Before the shell exists (no realize yet), the folder the pane was started in answers.
    #[test]
    fn initial_folder_answers_before_the_shell_exists() {
        let track = new_track();
        track.lock().unwrap().initial = Some(PathBuf::from("/"));
        assert_eq!(pane_cwd(u64::MAX, &track), Some(PathBuf::from("/")));
        // A reported folder still wins over the initial one.
        track.lock().unwrap().cwd = Some("/usr".to_string());
        assert_eq!(pane_cwd(u64::MAX, &track), Some(PathBuf::from("/usr")));
    }

    #[test]
    fn remote_pane_has_no_local_folder() {
        let track = new_track();
        {
            let mut t = track.lock().unwrap();
            t.cwd = Some("/".to_string());
            t.remote = true;
        }
        assert_eq!(pane_cwd(u64::MAX, &track), None);
    }
}
