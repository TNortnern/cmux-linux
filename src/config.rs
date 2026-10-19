use gtk4::gdk::{Key, ModifierType};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

/// Default scrollback per pane, in bytes. Ghostty's own default is 10 MB per pane.
pub const DEFAULT_SCROLLBACK_BYTES: u64 = 2_000_000;

/// Top-level config loaded from ~/.config/cmux/config.toml.
#[derive(serde::Deserialize, Default, Debug)]
pub struct Config {
    #[serde(default)]
    pub shortcuts: ShortcutConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
}

/// `[session]` -- workspaces, tabs and split layout are restored on the next launch.
/// Shells are not kept alive; each pane restarts in its saved folder.
#[derive(serde::Deserialize, Debug, Clone)]
#[serde(default)]
pub struct SessionConfig {
    /// Set to false to start with an empty window (the saved session is ignored).
    pub restore: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self { restore: true }
    }
}

/// `[terminal]` -- settings passed to the embedded ghostty engine.
#[derive(serde::Deserialize, Debug, Clone)]
#[serde(default)]
pub struct TerminalConfig {
    /// Scrollback kept per pane, in bytes. 0 disables scrollback.
    pub scrollback_bytes: u64,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self { scrollback_bytes: DEFAULT_SCROLLBACK_BYTES }
    }
}

/// Per-action shortcut overrides. Each value is a GTK accelerator string (e.g. "<Ctrl>n").
/// None means "use default".
#[derive(serde::Deserialize, Default, Debug)]
pub struct ShortcutConfig {
    pub new_workspace: Option<String>,
    pub close_workspace: Option<String>,
    pub next_workspace: Option<String>,
    pub prev_workspace: Option<String>,
    pub rename_workspace: Option<String>,
    pub toggle_sidebar: Option<String>,
    pub split_right: Option<String>,
    pub split_down: Option<String>,
    pub close_pane: Option<String>,
    pub new_ssh_workspace: Option<String>,
    pub focus_left: Option<String>,
    pub focus_right: Option<String>,
    pub focus_up: Option<String>,
    pub focus_down: Option<String>,
    pub workspace_1: Option<String>,
    pub workspace_2: Option<String>,
    pub workspace_3: Option<String>,
    pub workspace_4: Option<String>,
    pub workspace_5: Option<String>,
    pub workspace_6: Option<String>,
    pub workspace_7: Option<String>,
    pub workspace_8: Option<String>,
    pub workspace_9: Option<String>,
    pub browser_open: Option<String>,
    pub browser_close: Option<String>,
}

/// UI configuration section -- [ui] in config.toml (D-16).
#[derive(serde::Deserialize, Default, Debug)]
pub struct UiConfig {
    #[serde(default)]
    pub header_bar: HeaderBarConfig,
    /// GTK renderer: "auto" (default), "gl" or "cairo". "auto" picks cairo when the machine has
    /// no GPU render node (VMs, WSL, servers): GL is Mesa's software rasterizer there anyway, and
    /// drawing GTK's own widgets with cairo instead saved ~20% RSS. Requires restart.
    #[serde(default = "default_renderer")]
    pub renderer: String,
}

fn default_renderer() -> String {
    "auto".to_string()
}

/// Whether GTK should draw with cairo. `has_gpu_node` is whether /dev/dri/renderD* exists.
pub fn use_cairo_renderer(setting: &str, has_gpu_node: bool) -> bool {
    match setting {
        "cairo" => true,
        "gl" => false,
        _ => !has_gpu_node,
    }
}

/// Header bar configuration -- [ui.header_bar] in config.toml (D-16).
/// Requires app restart to take effect.
#[derive(serde::Deserialize, Debug)]
pub struct HeaderBarConfig {
    /// "gtk" (default, full header bar), "custom" (user-specified buttons), "none" (no header bar)
    #[serde(default = "default_header_style")]
    pub style: String,
    /// Button names for left side (only used when style="custom")
    pub buttons_left: Option<Vec<String>>,
    /// Button names for right side (only used when style="custom")
    pub buttons_right: Option<Vec<String>>,
}

fn default_header_style() -> String {
    "gtk".to_string()
}

impl Default for HeaderBarConfig {
    fn default() -> Self {
        Self {
            style: default_header_style(),
            buttons_left: None,
            buttons_right: None,
        }
    }
}

/// All bindable shortcut actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    NewWorkspace,
    CloseWorkspace,
    NextWorkspace,
    PrevWorkspace,
    RenameWorkspace,
    ToggleSidebar,
    SplitRight,
    SplitDown,
    ClosePane,
    NewSshWorkspace,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    Workspace1,
    Workspace2,
    Workspace3,
    Workspace4,
    Workspace5,
    Workspace6,
    Workspace7,
    Workspace8,
    Workspace9,
    BrowserOpen,
    BrowserClose,
}

/// HashMap-based shortcut lookup table built from config + defaults.
pub struct ShortcutMap {
    map: HashMap<(ModifierType, Key), ShortcutAction>,
}

/// Known shortcut action names for unknown-key detection.
const KNOWN_SHORTCUTS: &[&str] = &[
    "new_workspace", "close_workspace", "next_workspace", "prev_workspace",
    "rename_workspace", "toggle_sidebar", "split_right", "split_down",
    "close_pane", "new_ssh_workspace", "focus_left", "focus_right", "focus_up", "focus_down",
    "workspace_1", "workspace_2", "workspace_3", "workspace_4",
    "workspace_5", "workspace_6", "workspace_7", "workspace_8", "workspace_9",
    "browser_open", "browser_close",
];

/// Modifier mask for lookup: ignore Caps Lock, Num Lock, etc.
const MOD_MASK: ModifierType = ModifierType::from_bits_truncate(
    ModifierType::CONTROL_MASK.bits()
        | ModifierType::SHIFT_MASK.bits()
        | ModifierType::ALT_MASK.bits(),
);

/// Returns the config file path.
/// Respects $XDG_CONFIG_HOME/cmux/config.toml; falls back to ~/.config/cmux/config.toml (CFG-04).
pub fn config_path() -> PathBuf {
    config_path_from(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
}

/// Pure form of `config_path`, so the tests need not touch the process environment.
fn config_path_from(xdg_config_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = match xdg_config_home {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home.unwrap_or_else(|| OsString::from("."))).join(".config"),
    };
    base.join("cmux").join("config.toml")
}

/// Load config from disk. Always returns a usable Config (D-10).
/// A missing file is silent. A read error or TOML syntax error warns and uses all defaults.
/// A bad value only affects its own entry: it is reported and skipped, and the rest is kept.
pub fn load_config() -> Config {
    load_config_from(&config_path())
}

/// Load config from an explicit file path. See `load_config`.
fn load_config_from(path: &std::path::Path) -> Config {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Config::default();
        }
        Err(e) => {
            cmux_warn!("config read error at {}: {e}", path.display());
            return Config::default();
        }
    };
    parse_config(&content, path)
}

/// Parse config text. `path` is only used in messages.
fn parse_config(content: &str, path: &std::path::Path) -> Config {
    let root: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(e) => {
            cmux_warn!("config parse error at {}: {e}; using defaults", path.display());
            return Config::default();
        }
    };

    warn_unknown_shortcuts(&root);

    let ui = match sub_table(&root, "ui") {
        Some(ui) => UiConfig { header_bar: lenient(ui, "header_bar"), renderer: lenient(ui, "renderer") },
        None => UiConfig::default(),
    };

    Config {
        shortcuts: lenient(&root, "shortcuts"),
        ui,
        session: lenient(&root, "session"),
        terminal: lenient(&root, "terminal"),
    }
}

/// Returns `parent[name]` if it is a table. Otherwise warns (when set) and returns None.
fn sub_table<'a>(parent: &'a toml::Table, name: &str) -> Option<&'a toml::Table> {
    match parent.get(name)? {
        toml::Value::Table(t) => Some(t),
        _ => {
            cmux_warn!("config: [{name}] must be a table, ignoring it");
            None
        }
    }
}

/// Deserializes the table `parent[name]` one entry at a time. An entry that fails to
/// parse is reported and dropped. The other entries are kept. All target types have
/// defaults for every field, so a single-entry probe only fails on a bad value.
fn lenient<T: DeserializeOwned + Default>(parent: &toml::Table, name: &str) -> T {
    let Some(entries) = sub_table(parent, name) else {
        return T::default();
    };
    let mut good = toml::Table::new();
    for (key, value) in entries {
        let mut probe = toml::Table::new();
        probe.insert(key.clone(), value.clone());
        if toml::Value::Table(probe).try_into::<T>().is_ok() {
            good.insert(key.clone(), value.clone());
        } else {
            cmux_warn!("config: invalid value for '{name}.{key}', ignoring it");
        }
    }
    toml::Value::Table(good).try_into::<T>().unwrap_or_default()
}

/// Warn about unknown keys in the [shortcuts] table (D-03).
fn warn_unknown_shortcuts(root: &toml::Table) {
    if let Some(shortcuts) = root.get("shortcuts").and_then(|v| v.as_table()) {
        for key in shortcuts.keys() {
            if !KNOWN_SHORTCUTS.contains(&key.as_str()) {
                cmux_warn!("unknown shortcut action '{}' in config, ignoring", key);
            }
        }
    }
}

impl ShortcutMap {
    /// Build lookup table from config, falling back to defaults for unset/invalid entries.
    pub fn from_config(config: &ShortcutConfig) -> Self {
        let entries: &[(ShortcutAction, &Option<String>, &str)] = &[
            (ShortcutAction::NewWorkspace,    &config.new_workspace,    "<Ctrl>n"),
            (ShortcutAction::CloseWorkspace,  &config.close_workspace,  "<Ctrl><Shift>w"),
            (ShortcutAction::NextWorkspace,   &config.next_workspace,   "<Ctrl>bracketright"),
            (ShortcutAction::PrevWorkspace,   &config.prev_workspace,   "<Ctrl>bracketleft"),
            (ShortcutAction::RenameWorkspace, &config.rename_workspace, "<Ctrl><Shift>r"),
            (ShortcutAction::ToggleSidebar,   &config.toggle_sidebar,   "<Ctrl>b"),
            (ShortcutAction::SplitRight,      &config.split_right,      "<Ctrl>d"),
            (ShortcutAction::SplitDown,       &config.split_down,       "<Ctrl><Shift>d"),
            (ShortcutAction::ClosePane,       &config.close_pane,       "<Ctrl><Shift>x"),
            (ShortcutAction::NewSshWorkspace, &config.new_ssh_workspace, "<Ctrl><Shift>s"),
            (ShortcutAction::FocusLeft,       &config.focus_left,       "<Ctrl><Shift>Left"),
            (ShortcutAction::FocusRight,      &config.focus_right,      "<Ctrl><Shift>Right"),
            (ShortcutAction::FocusUp,         &config.focus_up,         "<Ctrl><Shift>Up"),
            (ShortcutAction::FocusDown,       &config.focus_down,       "<Ctrl><Shift>Down"),
            (ShortcutAction::Workspace1,      &config.workspace_1,      "<Ctrl>1"),
            (ShortcutAction::Workspace2,      &config.workspace_2,      "<Ctrl>2"),
            (ShortcutAction::Workspace3,      &config.workspace_3,      "<Ctrl>3"),
            (ShortcutAction::Workspace4,      &config.workspace_4,      "<Ctrl>4"),
            (ShortcutAction::Workspace5,      &config.workspace_5,      "<Ctrl>5"),
            (ShortcutAction::Workspace6,      &config.workspace_6,      "<Ctrl>6"),
            (ShortcutAction::Workspace7,      &config.workspace_7,      "<Ctrl>7"),
            (ShortcutAction::Workspace8,      &config.workspace_8,      "<Ctrl>8"),
            (ShortcutAction::Workspace9,      &config.workspace_9,      "<Ctrl>9"),
            (ShortcutAction::BrowserOpen,     &config.browser_open,     "<Ctrl><Shift>b"),
            (ShortcutAction::BrowserClose,    &config.browser_close,    "<Ctrl><Shift>q"),
        ];

        let mut map = HashMap::new();

        for (action, config_val, default_accel) in entries {
            let accel_str = config_val.as_deref().unwrap_or(*default_accel);
            let action_name = format!("{:?}", action);

            if let Some((key, mods)) = gtk4::accelerator_parse(accel_str) {
                map.insert((mods & MOD_MASK, key), *action);
            } else {
                // D-11: invalid accelerator — warn and use default
                cmux_warn!(
                    "invalid shortcut '{}' for {}, using default '{}'",
                    accel_str, action_name, default_accel
                );
                if let Some((key, mods)) = gtk4::accelerator_parse(*default_accel) {
                    map.insert((mods & MOD_MASK, key), *action);
                }
            }
        }

        ShortcutMap { map }
    }

    /// Look up a shortcut action for the given modifier+key combination.
    /// Masks modifiers to ignore Caps Lock, Num Lock, etc.
    /// Normalizes keyval to lowercase because GTK4 key events give uppercase
    /// when Shift is held (e.g. Key::R), but accelerator_parse stores lowercase
    /// with the Shift modifier flag (e.g. Key::r + SHIFT_MASK).
    pub fn lookup(&self, mods: ModifierType, key: Key) -> Option<ShortcutAction> {
        let masked = mods & MOD_MASK;
        let lower_key = key.to_lower();
        self.map.get(&(masked, lower_key)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path_from_xdg() {
        let path = config_path_from(Some("/tmp/test-xdg-config".into()), Some("/home/x".into()));
        assert_eq!(path, PathBuf::from("/tmp/test-xdg-config/cmux/config.toml"));
    }

    #[test]
    fn test_config_path_from_home_fallback() {
        let path = config_path_from(None, Some("/home/x".into()));
        assert_eq!(path, PathBuf::from("/home/x/.config/cmux/config.toml"));
    }

    /// A unique scratch dir per test, so tests can run in parallel.
    fn scratch_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cmux-cfg-{name}-{}", std::process::id()))
    }

    /// Writes `content` to `<dir>/config.toml` and returns that path.
    fn write_config(dir: &std::path::Path, content: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, content).unwrap();
        file
    }

    #[test]
    fn test_load_config_missing_file() {
        // A path that does not exist: load_config_from returns defaults silently.
        let config = load_config_from(&scratch_dir("missing").join("config.toml"));
        assert!(config.shortcuts.new_workspace.is_none());
        assert!(config.shortcuts.close_workspace.is_none());
    }

    #[test]
    fn test_load_config_empty_file() {
        let dir = scratch_dir("empty");
        let file = write_config(&dir, "");
        let config = load_config_from(&file);
        assert!(config.shortcuts.new_workspace.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_config_valid_shortcuts() {
        let dir = scratch_dir("valid");
        let file = write_config(&dir, "[shortcuts]\nnew_workspace = \"<Ctrl>t\"\n");
        let config = load_config_from(&file);
        assert_eq!(config.shortcuts.new_workspace, Some("<Ctrl>t".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_config_invalid_toml() {
        let dir = scratch_dir("invalid");
        let file = write_config(&dir, "[shortcuts\n");
        // Falls back to defaults on a syntax error
        let config = load_config_from(&file);
        assert!(config.shortcuts.new_workspace.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ui_config_default() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.ui.header_bar.style, "gtk");
        assert!(config.ui.header_bar.buttons_left.is_none());
    }

    #[test]
    fn test_ui_config_custom_style() {
        let config: Config = toml::from_str(r#"
[ui.header_bar]
style = "none"
buttons_left = ["new_workspace"]
buttons_right = ["split_right", "toggle_sidebar"]
"#).unwrap();
        assert_eq!(config.ui.header_bar.style, "none");
        assert_eq!(config.ui.header_bar.buttons_left.as_ref().unwrap().len(), 1);
        assert_eq!(config.ui.header_bar.buttons_right.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_session_and_terminal_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.session.restore);
        assert_eq!(config.terminal.scrollback_bytes, 2_000_000);
    }

    #[test]
    fn test_session_and_terminal_override() {
        let config = parse_config(
            "[session]\nrestore = false\n[terminal]\nscrollback_bytes = 500000\n",
            std::path::Path::new("test"),
        );
        assert!(!config.session.restore);
        assert_eq!(config.terminal.scrollback_bytes, 500_000);
    }

    #[test]
    fn test_bad_entry_keeps_other_entries() {
        let config = parse_config(
            "[shortcuts]\nnew_workspace = 42\nclose_workspace = \"<Ctrl>w\"\n\
             [session]\nrestore = \"yes please\"\n\
             [terminal]\nscrollback_bytes = 1000\n\
             [ui.header_bar]\nstyle = \"none\"\n",
            std::path::Path::new("test"),
        );
        // The bad shortcut is dropped, the good one is kept.
        assert!(config.shortcuts.new_workspace.is_none());
        assert_eq!(config.shortcuts.close_workspace.as_deref(), Some("<Ctrl>w"));
        // The bad session value falls back to its default; the other sections load.
        assert!(config.session.restore);
        assert_eq!(config.terminal.scrollback_bytes, 1000);
        assert_eq!(config.ui.header_bar.style, "none");
    }

    #[test]
    fn test_non_table_section_is_ignored() {
        let config = parse_config("shortcuts = 3\n[terminal]\nscrollback_bytes = 7\n", std::path::Path::new("test"));
        assert!(config.shortcuts.new_workspace.is_none());
        assert_eq!(config.terminal.scrollback_bytes, 7);
    }

    // Tests that require GTK4 initialization (accelerator_parse).
    // These will only work in environments with a display (or virtual display).

    // One test, because GTK can only be initialized on one test thread per process.
    // A second gtk4::init() call from another thread panics.
    #[test]
    fn test_shortcut_map() {
        if gtk4::init().is_err() {
            eprintln!("Skipping test_shortcut_map: GTK4 init failed (headless)");
            return;
        }
        // Defaults: Ctrl+N should map to NewWorkspace
        let smap = ShortcutMap::from_config(&ShortcutConfig::default());
        assert_eq!(smap.lookup(ModifierType::CONTROL_MASK, Key::n), Some(ShortcutAction::NewWorkspace));

        // Custom: Ctrl+T maps to NewWorkspace and Ctrl+N no longer does
        let config = ShortcutConfig {
            new_workspace: Some("<Ctrl>t".to_string()),
            ..Default::default()
        };
        let smap = ShortcutMap::from_config(&config);
        assert_eq!(smap.lookup(ModifierType::CONTROL_MASK, Key::t), Some(ShortcutAction::NewWorkspace));
        assert_eq!(smap.lookup(ModifierType::CONTROL_MASK, Key::n), None);
    }

    #[test]
    fn renderer_auto_uses_cairo_only_without_gpu() {
        assert!(super::use_cairo_renderer("auto", false));
        assert!(!super::use_cairo_renderer("auto", true));
        assert!(super::use_cairo_renderer("cairo", true));
        assert!(!super::use_cairo_renderer("gl", false));
    }
}
