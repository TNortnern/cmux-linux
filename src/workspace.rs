use crate::session::{self, TabSession};
use crate::split_engine::{SplitEngine, SplitNodeData};
use crate::tab_bar::TabBar;
use gtk4::prelude::*;
use std::path::PathBuf;
use uuid::Uuid;

/// Connection state for SSH remote workspaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not a remote workspace (local).
    Local,
    /// SSH connection established and healthy.
    Connected,
    /// SSH connection lost.
    Disconnected,
    /// Attempting to reconnect (with attempt count).
    Reconnecting(u32),
}

impl ConnectionState {
    pub fn is_remote(&self) -> bool {
        !matches!(self, ConnectionState::Local)
    }

    pub fn display_text(&self) -> &str {
        match self {
            ConnectionState::Local => "",
            ConnectionState::Connected => "Connected",
            ConnectionState::Disconnected => "Disconnected",
            ConnectionState::Reconnecting(_) => "Reconnecting...",
        }
    }

    pub fn css_class(&self) -> &str {
        match self {
            ConnectionState::Local => "",
            ConnectionState::Connected => "connected",
            ConnectionState::Disconnected => "disconnected",
            ConnectionState::Reconnecting(_) => "reconnecting",
        }
    }
}

// -- Tab index math. Pure functions, so the tab operations can be tested without GTK. --

/// Tab to select after `active`, wrapping from the last tab to the first.
pub fn next_tab_index(len: usize, active: usize) -> usize {
    if len == 0 {
        0
    } else {
        (active + 1) % len
    }
}

/// Tab to select before `active`, wrapping from the first tab to the last.
pub fn prev_tab_index(len: usize, active: usize) -> usize {
    if len == 0 {
        0
    } else if active == 0 {
        len - 1
    } else {
        active - 1
    }
}

/// Index of the selected tab after the tab at `from` is moved to `to` (remove, then insert).
pub fn active_after_move(active: usize, from: usize, to: usize) -> usize {
    if active == from {
        to
    } else if from < active && to >= active {
        active - 1
    } else if from > active && to <= active {
        active + 1
    } else {
        active
    }
}

/// Index of the selected tab after the tab at `closed` is removed from `len` tabs (`len` counts
/// the closed tab). Closing the selected tab selects its right neighbour, or the left one when
/// it was the last tab.
pub fn active_after_close(len: usize, active: usize, closed: usize) -> usize {
    if closed < active {
        active - 1
    } else if closed > active {
        active
    } else if closed + 1 < len {
        closed
    } else {
        closed.saturating_sub(1)
    }
}

/// A saved layout that has not been built yet (lazy restore).
#[derive(Debug, Clone)]
pub struct SavedLayout {
    pub layout: SplitNodeData,
    pub active_pane_uuid: Option<String>,
}

/// One tab: a named split tree. `engine` is None until the tab is first shown; `saved` holds the
/// layout to build at that point. A tab has exactly one of the two.
pub struct Tab {
    /// Process-unique id, used for the GtkStack page name and for tab-bar callbacks.
    pub id: u64,
    pub name: String,
    pub engine: Option<SplitEngine>,
    pub saved: Option<SavedLayout>,
}

impl Tab {
    pub fn live(id: u64, name: String, engine: SplitEngine) -> Self {
        Self { id, name, engine: Some(engine), saved: None }
    }

    pub fn lazy(id: u64, name: String, saved: SavedLayout) -> Self {
        Self { id, name, engine: None, saved: Some(saved) }
    }

    /// Folder for a pane opened from this tab: the focused pane's folder when the tab is live,
    /// else the saved folder of its focused pane.
    pub fn cwd(&self) -> Option<PathBuf> {
        match (&self.engine, &self.saved) {
            (Some(engine), _) => engine.focused_cwd(),
            (None, Some(saved)) => session::layout_cwd(&saved.layout, saved.active_pane_uuid.as_deref()),
            (None, None) => None,
        }
    }

    /// Session entry for this tab, or None if it has no terminal to save.
    pub fn session_entry(&self) -> Option<TabSession> {
        match (&self.engine, &self.saved) {
            (Some(engine), _) => Some(TabSession {
                name: self.name.clone(),
                active_pane_uuid: engine.active_pane_uuid(),
                layout: engine.root.to_session_data()?,
            }),
            (None, Some(saved)) => Some(TabSession {
                name: self.name.clone(),
                active_pane_uuid: saved.active_pane_uuid.clone(),
                layout: saved.layout.clone(),
            }),
            (None, None) => None,
        }
    }
}

/// GTK widgets of one workspace. Layout: root Box(V) > [tab_bar, tab_stack]. The stack has one
/// page per tab, and each page is the tab's split-tree root (or a placeholder while lazy).
pub struct WorkspaceUi {
    pub root: gtk4::Box,
    pub tab_stack: gtk4::Stack,
    pub tab_bar: TabBar,
}

impl WorkspaceUi {
    pub fn new(ws_id: u64) -> Self {
        let tab_bar = TabBar::new(ws_id);
        let tab_stack = gtk4::Stack::new();
        tab_stack.set_transition_type(gtk4::StackTransitionType::None);
        tab_stack.set_hexpand(true);
        tab_stack.set_vexpand(true);
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.append(&tab_bar.root);
        root.append(&tab_stack);
        Self { root, tab_stack, tab_bar }
    }
}

/// Workspace: one entry in the cmux sidebar. Holds its tabs, and each tab holds one split tree.
/// The workspace's `ui.root` is the page for it in the top-level GtkStack.
pub struct Workspace {
    /// Unique workspace ID.
    pub id: u64,
    /// Display name shown in the sidebar GtkListBox row.
    pub name: String,
    /// The name key used with GtkStack::add_named / set_visible_child_name.
    pub stack_page_name: String,
    /// Sequential number used for default naming ("Workspace N").
    /// Preserved even after renames so we don't reuse numbers.
    pub display_number: usize,
    /// Stable UUID for session persistence and v2 socket protocol identity.
    pub uuid: Uuid,
    /// Phase 4 NOTF-01: true when any pane in this workspace has unread bell activity.
    pub has_attention: bool,
    /// Phase 4: rate-limit desktop notifications to 1 per workspace per 5 seconds.
    pub last_notification: Option<std::time::Instant>,
    /// SSH remote target (e.g., "user@host"). None for local workspaces.
    pub remote_target: Option<String>,
    /// Connection state for remote workspaces.
    pub connection_state: ConnectionState,
    /// Tabs in tab-bar order. A workspace always has at least one.
    pub tabs: Vec<Tab>,
    /// Index into `tabs` of the selected tab.
    pub active_tab: usize,
    /// Next number for default tab names ("Tab N"). Names are never reused within a workspace.
    next_tab_number: usize,
    pub ui: WorkspaceUi,
}

impl Workspace {
    /// Create a new workspace with a default "Workspace N" name and no tabs yet.
    pub fn new(id: u64, display_number: usize, ui: WorkspaceUi) -> Self {
        Self {
            id,
            name: format!("Workspace {}", display_number),
            stack_page_name: format!("workspace-{}", id),
            display_number,
            uuid: Uuid::new_v4(),
            has_attention: false,
            last_notification: None,
            remote_target: None,
            connection_state: ConnectionState::Local,
            tabs: Vec::new(),
            active_tab: 0,
            next_tab_number: 1,
            ui,
        }
    }

    /// Rename this workspace to a new display name.
    pub fn rename(&mut self, new_name: String) {
        self.name = new_name;
    }

    /// Create a new remote SSH workspace targeting the given host, with no tabs yet.
    pub fn new_remote(id: u64, display_number: usize, target: String, ui: WorkspaceUi) -> Self {
        let mut ws = Self::new(id, display_number, ui);
        ws.name = crate::ssh_hosts::workspace_name_from_target(&target);
        ws.remote_target = Some(target);
        ws.connection_state = ConnectionState::Reconnecting(0);
        ws
    }

    /// Split tree of the selected tab, if it is built.
    pub fn active_engine(&self) -> Option<&SplitEngine> {
        self.tabs.get(self.active_tab)?.engine.as_ref()
    }

    pub fn active_engine_mut(&mut self) -> Option<&mut SplitEngine> {
        self.tabs.get_mut(self.active_tab)?.engine.as_mut()
    }

    /// Position of the tab with this id.
    pub fn tab_index(&self, tab_id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == tab_id)
    }

    /// Next default tab name ("Tab N"). Skips numbers already taken by restored tabs.
    pub fn take_default_tab_name(&mut self) -> String {
        let n = self.next_tab_number.max(self.tabs.len() + 1);
        self.next_tab_number = n + 1;
        format!("Tab {n}")
    }

    /// True if a pane in any built tab has unread bell activity.
    pub fn any_attention(&self) -> bool {
        self.tabs
            .iter()
            .filter_map(|t| t.engine.as_ref())
            .any(|e| e.root.any_attention())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moves tab `from` to `to` in a list of `len` tabs labelled 0..len, and returns the position
    /// of the tab that was labelled `active` before the move.
    fn after_move(len: usize, active: usize, from: usize, to: usize) -> usize {
        let mut tabs: Vec<usize> = (0..len).collect();
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        tabs.iter().position(|&t| t == active).unwrap()
    }

    #[test]
    fn next_and_prev_wrap() {
        assert_eq!(next_tab_index(3, 0), 1);
        assert_eq!(next_tab_index(3, 2), 0);
        assert_eq!(prev_tab_index(3, 1), 0);
        assert_eq!(prev_tab_index(3, 0), 2);
        // A single tab stays selected.
        assert_eq!(next_tab_index(1, 0), 0);
        assert_eq!(prev_tab_index(1, 0), 0);
        // No tabs: index 0, no panic.
        assert_eq!(next_tab_index(0, 0), 0);
        assert_eq!(prev_tab_index(0, 0), 0);
    }

    #[test]
    fn move_tab_keeps_selection_on_the_same_tab() {
        for len in 1..=6 {
            for from in 0..len {
                for to in 0..len {
                    for active in 0..len {
                        let expected = after_move(len, active, from, to);
                        assert_eq!(
                            active_after_move(active, from, to),
                            expected,
                            "len={len} active={active} from={from} to={to}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn closing_active_tab_selects_right_neighbour_or_left_when_last() {
        // [A B C], select B, close B -> C (now index 1).
        assert_eq!(active_after_close(3, 1, 1), 1);
        // [A B C], select C, close C -> B (index 1, the new last tab).
        assert_eq!(active_after_close(3, 2, 2), 1);
        // [A B C], select A, close A -> B (index 0).
        assert_eq!(active_after_close(3, 0, 0), 0);
    }

    #[test]
    fn closing_other_tabs_keeps_selection_on_the_same_tab() {
        // [A B C], select C, close A -> C is now index 1.
        assert_eq!(active_after_close(3, 2, 0), 1);
        // [A B C], select A, close C -> A unchanged.
        assert_eq!(active_after_close(3, 0, 2), 0);
    }

    #[test]
    fn close_index_matches_list_removal() {
        for len in 2..=6 {
            for closed in 0..len {
                for active in 0..len {
                    let mut tabs: Vec<usize> = (0..len).collect();
                    tabs.remove(closed);
                    let new_active = active_after_close(len, active, closed);
                    if closed == active {
                        // The selected tab is gone: the choice must be a neighbour, not out of range.
                        assert!(new_active < len - 1, "len={len} closed={closed}");
                    } else {
                        assert_eq!(
                            tabs[new_active], active,
                            "len={len} active={active} closed={closed}"
                        );
                    }
                }
            }
        }
    }
}
