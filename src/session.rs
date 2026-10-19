use crate::split_engine::SplitNodeData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Schema written by this build. Schema 1 (one layout per workspace) is still read and migrated.
pub const SESSION_VERSION: u32 = 2;

/// Serializes writers. The debounced save task and the quit-time flush can run at the same time.
static SAVE_LOCK: Mutex<()> = Mutex::new(());

/// One tab: its name, its focused pane and its split tree.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct TabSession {
    pub name: String,
    /// UUID of the focused pane in this tab, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_pane_uuid: Option<String>,
    pub layout: SplitNodeData,
}

/// One workspace: its tabs in tab-bar order and which of them is selected.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WorkspaceSession {
    /// Workspace UUID, stable across restarts.
    pub id: String,
    pub name: String,
    /// Index into `tabs` of the selected tab.
    #[serde(default)]
    pub selected_tab: usize,
    pub tabs: Vec<TabSession>,
}

/// Root session data written to session.json.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SessionData {
    pub version: u32,
    /// Index into `workspaces` of the selected workspace.
    #[serde(default)]
    pub selected_workspace: usize,
    pub workspaces: Vec<WorkspaceSession>,
}

/// Schema 1 workspace: one layout, no tabs. Read only.
#[derive(serde::Deserialize)]
struct WorkspaceSessionV1 {
    uuid: String,
    name: String,
    #[serde(default)]
    active_pane_uuid: Option<String>,
    layout: SplitNodeData,
}

/// Schema 1 root. Read only.
#[derive(serde::Deserialize)]
struct SessionDataV1 {
    #[serde(default)]
    active_index: usize,
    workspaces: Vec<WorkspaceSessionV1>,
}

/// Each v1 workspace becomes a workspace with one tab, which is selected.
fn migrate_v1(v1: SessionDataV1) -> SessionData {
    SessionData {
        version: SESSION_VERSION,
        selected_workspace: v1.active_index,
        workspaces: v1
            .workspaces
            .into_iter()
            .map(|ws| WorkspaceSession {
                id: ws.uuid,
                name: ws.name,
                selected_tab: 0,
                tabs: vec![TabSession {
                    name: "Tab 1".to_string(),
                    active_pane_uuid: ws.active_pane_uuid,
                    layout: ws.layout,
                }],
            })
            .collect(),
    }
}

/// Parses session JSON of schema 1 or 2. Schema 1 is migrated to schema 2.
pub fn parse_session(content: &str) -> Result<SessionData, String> {
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("session JSON invalid: {e}"))?;
    match value.get("version").and_then(|v| v.as_u64()) {
        Some(1) => serde_json::from_value::<SessionDataV1>(value)
            .map(migrate_v1)
            .map_err(|e| format!("session v1 invalid: {e}")),
        Some(2) => serde_json::from_value::<SessionData>(value)
            .map_err(|e| format!("session v2 invalid: {e}")),
        other => Err(format!("session version {other:?} not supported")),
    }
}

/// Returns the session file path.
/// Respects $XDG_DATA_HOME/cmux/session.json; falls back to ~/.local/share/cmux/session.json.
pub fn session_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/.local/share")
    });
    PathBuf::from(base).join("cmux").join("session.json")
}

/// Set by `save_session_final`. Once set, debounced saves are skipped, so a snapshot that was
/// taken before quit cannot land after the final one.
static FINAL_WRITTEN: AtomicBool = AtomicBool::new(false);

/// Debounced save from the background task. Skipped once the final state has been written.
pub fn save_session_debounced(data: &SessionData) -> std::io::Result<()> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if FINAL_WRITTEN.load(Ordering::SeqCst) {
        return Ok(());
    }
    write_session_locked(data, &session_path())
}

/// Quit-time save. Writes the final state and marks the session as final, so no later debounced
/// save overwrites it. Both steps happen under the save lock.
pub fn save_session_final(data: &SessionData) -> std::io::Result<()> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    FINAL_WRITTEN.store(true, Ordering::SeqCst);
    write_session_locked(data, &session_path())
}

/// Save to a specific path, atomically: written to session.json.tmp, then renamed over the
/// target. rename() is atomic on Linux (same filesystem), so a crash leaves only the .tmp file.
#[cfg(test)]
pub fn save_session_to(data: &SessionData, path: &Path) -> std::io::Result<()> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    write_session_locked(data, path)
}

/// Writes the session file. The caller holds SAVE_LOCK.
fn write_session_locked(data: &SessionData, path: &Path) -> std::io::Result<()> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)
        .map_err(std::io::Error::other)?;
    write_private(&tmp_path, json.as_bytes())?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Writes `bytes` to a new file at `path` with mode 0600. The session records working
/// directories, so other users must not read it. A stale file at `path` (left by a crash)
/// is removed first, so the mode always applies.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Load session from disk. Returns None if the file is missing, empty, or invalid JSON.
/// Never panics -- always returns a usable result for graceful fallback (SESS-04).
pub fn load_session() -> Option<SessionData> {
    load_session_from(&session_path())
}

/// Internal: load from a specific path (used in tests with temp paths).
pub fn load_session_from(path: &Path) -> Option<SessionData> {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            cmux_debug!("no session file at {}", path.display());
            return None;
        }
        Err(e) => {
            cmux_warn!("session file read error: {e}");
            return None;
        }
    };
    match parse_session(&content) {
        Ok(data) => Some(data),
        Err(e) => {
            cmux_warn!("{e}, ignoring");
            None
        }
    }
}

/// Folder to start a pane in, taken from a saved layout: the focused leaf's folder if it is
/// usable, else the first leaf with a usable folder. A folder is usable when it is an existing
/// absolute directory.
pub fn layout_cwd(layout: &SplitNodeData, active_uuid: Option<&str>) -> Option<PathBuf> {
    let mut leaves = Vec::new();
    collect_leaves(layout, &mut leaves);
    let focused = active_uuid.and_then(|a| leaves.iter().find(|(u, _)| u.as_str() == a));
    focused
        .into_iter()
        .chain(leaves.iter())
        .find_map(|(_, cwd)| {
            let path = PathBuf::from(cwd);
            (path.is_absolute() && path.is_dir()).then_some(path)
        })
}

/// (surface uuid, cwd) of every leaf, in layout order.
fn collect_leaves<'a>(node: &'a SplitNodeData, out: &mut Vec<(String, &'a str)>) {
    match node {
        SplitNodeData::Leaf { surface_uuid, cwd, .. } => {
            out.push((surface_uuid.to_string(), cwd.as_str()));
        }
        SplitNodeData::Split { start, end, .. } => {
            collect_leaves(start, out);
            collect_leaves(end, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const ALPHA_PANE: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    fn leaf(surface: &str, cwd: &str) -> SplitNodeData {
        SplitNodeData::Leaf {
            pane_id: 1,
            surface_uuid: Uuid::parse_str(surface).unwrap(),
            shell: "/bin/sh".to_string(),
            cwd: cwd.to_string(),
        }
    }

    fn dummy_session(name: &str) -> SessionData {
        SessionData {
            version: SESSION_VERSION,
            selected_workspace: 0,
            workspaces: vec![WorkspaceSession {
                id: "test-uuid-1".to_string(),
                name: name.to_string(),
                selected_tab: 0,
                tabs: vec![TabSession {
                    name: "Tab 1".to_string(),
                    active_pane_uuid: None,
                    layout: leaf(ALPHA_PANE, "/tmp"),
                }],
            }],
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cmux-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// SESS-01: save_session_to must write session.json to disk for valid data.
    /// Verifies the full trigger -> write path, not just Ok(()) return.
    #[test]
    fn test_save_triggered() {
        let dir = temp_dir("save");
        let path = dir.join("session.json");
        save_session_to(&dummy_session("TestWorkspace"), &path).expect("save failed");
        assert!(path.exists(), "session.json not created on disk after save_session_to");
        let content = std::fs::read_to_string(&path).expect("could not read session.json");
        let parsed: SessionData = serde_json::from_str(&content).expect("session.json is not valid JSON");
        assert_eq!(parsed.workspaces[0].name, "TestWorkspace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SESS-02: save then load reproduces workspaces, tabs and selection.
    #[test]
    fn test_restore_roundtrip() {
        let dir = temp_dir("rt");
        let path = dir.join("session.json");
        let mut data = dummy_session("MyWorkspace");
        data.workspaces[0].tabs.push(TabSession {
            name: "Logs".to_string(),
            active_pane_uuid: Some(ALPHA_PANE.to_string()),
            layout: leaf(ALPHA_PANE, "/usr"),
        });
        data.workspaces[0].selected_tab = 1;
        save_session_to(&data, &path).expect("save failed");

        let loaded = load_session_from(&path).expect("load returned None");
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.workspaces.len(), 1);
        let ws = &loaded.workspaces[0];
        assert_eq!(ws.name, "MyWorkspace");
        assert_eq!(ws.selected_tab, 1);
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.tabs[1].name, "Logs");
        assert_eq!(ws.tabs[1].active_pane_uuid.as_deref(), Some(ALPHA_PANE));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SESS-03: Atomic write -- the .tmp file is gone after a successful rename.
    #[test]
    fn test_atomic_write() {
        let dir = temp_dir("atomic");
        let path = dir.join("session.json");
        let tmp_path = path.with_extension("json.tmp");
        save_session_to(&dummy_session("AtomicTest"), &path).unwrap();
        assert!(path.exists(), "session.json must exist after save");
        assert!(!tmp_path.exists(), "session.json.tmp must be gone after successful rename");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Session files record working directories, so they must be private to the user.
    #[test]
    fn session_file_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("perm");
        let path = dir.join("session.json");
        // A stale, world-readable file from an earlier run must not keep its mode.
        std::fs::write(&path, b"stale").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        save_session_to(&dummy_session("PermTest"), &path).expect("save failed");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "session.json must be readable only by its owner");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SESS-04: load_session returns None for missing file without panic.
    #[test]
    fn test_graceful_fallback() {
        let path = PathBuf::from("/tmp/cmux-nonexistent-session-xyz.json");
        assert!(load_session_from(&path).is_none());
    }

    /// Schema 1 (one layout per workspace) migrates to schema 2 with one tab per workspace.
    #[test]
    fn v1_session_migrates_to_one_tab_per_workspace() {
        let v1 = r#"{"version":1,"active_index":1,"workspaces":[
            {"uuid":"11111111-1111-4111-8111-111111111111","name":"Alpha",
             "active_pane_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
             "layout":{"type":"Leaf","pane_id":1,"surface_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","shell":"/bin/sh","cwd":"/tmp"}},
            {"uuid":"22222222-2222-4222-8222-222222222222","name":"Beta","active_pane_uuid":null,
             "layout":{"type":"Split","orientation":"horizontal","ratio":0.25,
               "start":{"type":"Leaf","pane_id":2,"surface_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","shell":"/bin/sh","cwd":"/usr"},
               "end":{"type":"Leaf","pane_id":3,"surface_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","shell":"/bin/sh","cwd":"/var"}}}
        ]}"#;
        let data = parse_session(v1).expect("v1 must parse");
        assert_eq!(data.version, 2);
        assert_eq!(data.selected_workspace, 1);
        assert_eq!(data.workspaces.len(), 2);

        let alpha = &data.workspaces[0];
        assert_eq!(alpha.id, "11111111-1111-4111-8111-111111111111");
        assert_eq!(alpha.name, "Alpha");
        assert_eq!(alpha.selected_tab, 0);
        assert_eq!(alpha.tabs.len(), 1);
        assert_eq!(alpha.tabs[0].name, "Tab 1");
        assert_eq!(alpha.tabs[0].active_pane_uuid.as_deref(), Some(ALPHA_PANE));

        let beta = &data.workspaces[1];
        assert_eq!(beta.tabs.len(), 1);
        assert!(beta.tabs[0].active_pane_uuid.is_none());
        match &beta.tabs[0].layout {
            SplitNodeData::Split { start, end, .. } => {
                assert!(matches!(**start, SplitNodeData::Leaf { ref cwd, .. } if cwd == "/usr"));
                assert!(matches!(**end, SplitNodeData::Leaf { ref cwd, .. } if cwd == "/var"));
            }
            other => panic!("expected split layout, got {other:?}"),
        }
    }

    /// The schema 2 file format, as written by save_session_to.
    #[test]
    fn v2_json_has_documented_shape() {
        let data = SessionData {
            version: 2,
            selected_workspace: 0,
            workspaces: vec![WorkspaceSession {
                id: "33333333-3333-4333-8333-333333333333".to_string(),
                name: "Work".to_string(),
                selected_tab: 0,
                tabs: vec![TabSession {
                    name: "Tab 1".to_string(),
                    active_pane_uuid: None,
                    layout: leaf(ALPHA_PANE, "/tmp"),
                }],
            }],
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["version"], 2);
        assert_eq!(json["selected_workspace"], 0);
        let ws = &json["workspaces"][0];
        assert_eq!(ws["id"], "33333333-3333-4333-8333-333333333333");
        assert_eq!(ws["selected_tab"], 0);
        assert_eq!(ws["tabs"][0]["name"], "Tab 1");
        // No active pane: the key is omitted rather than written as null.
        assert!(ws["tabs"][0].get("active_pane_uuid").is_none());
        assert_eq!(ws["tabs"][0]["layout"]["type"], "Leaf");
    }

    /// Schema 2 round-trips through JSON without changing any field.
    #[test]
    fn v2_serde_roundtrip_is_stable() {
        let data = dummy_session("Stable");
        let first = serde_json::to_string(&data).unwrap();
        let parsed = parse_session(&first).expect("v2 must parse");
        let second = serde_json::to_string(&parsed).unwrap();
        assert_eq!(first, second);
    }

    /// Optional v2 fields may be absent: selected_tab and selected_workspace default to 0.
    #[test]
    fn v2_optional_fields_default() {
        let json = r#"{"version":2,"workspaces":[{"id":"x","name":"n","tabs":[
            {"name":"t","layout":{"type":"Leaf","pane_id":1,"surface_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","shell":"","cwd":""}}]}]}"#;
        let data = parse_session(json).expect("must parse");
        assert_eq!(data.selected_workspace, 0);
        assert_eq!(data.workspaces[0].selected_tab, 0);
        assert!(data.workspaces[0].tabs[0].active_pane_uuid.is_none());
    }

    #[test]
    fn unknown_version_is_rejected() {
        assert!(parse_session(r#"{"version":3,"workspaces":[]}"#).is_err());
        assert!(parse_session(r#"{"workspaces":[]}"#).is_err());
        assert!(parse_session("not json").is_err());
    }

    /// The focused leaf's folder wins; an unusable one falls back to the next leaf.
    #[test]
    fn layout_cwd_prefers_focused_leaf() {
        let layout = SplitNodeData::Split {
            orientation: "horizontal".to_string(),
            ratio: 0.5,
            start: Box::new(leaf(ALPHA_PANE, "/usr")),
            end: Box::new(leaf("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "/var")),
        };
        assert_eq!(
            layout_cwd(&layout, Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")),
            Some(PathBuf::from("/var"))
        );
        assert_eq!(layout_cwd(&layout, None), Some(PathBuf::from("/usr")));

        let relative = leaf(ALPHA_PANE, "relative/path");
        assert_eq!(layout_cwd(&relative, Some(ALPHA_PANE)), None);
        let missing = leaf(ALPHA_PANE, "/nonexistent-cmux-dir-xyz");
        assert_eq!(layout_cwd(&missing, None), None);
    }
}
