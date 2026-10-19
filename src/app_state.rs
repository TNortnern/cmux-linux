use crate::ghostty::ffi;
use crate::session::{self, SessionData, WorkspaceSession, SESSION_VERSION};
use crate::split_engine::{alloc_pane_id, CloseResult, SplitEngine};
use crate::workspace::{self, ConnectionState, SavedLayout, Tab, Workspace, WorkspaceUi};
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use uuid::Uuid;

pub type AppStateRef = Rc<RefCell<AppState>>;

thread_local! {
    /// The app's AppState, for callbacks that only carry an id (ghostty's close callback, tab bar).
    /// Set by AppState::new. Main thread only.
    static APP_STATE: RefCell<Weak<RefCell<AppState>>> = RefCell::new(Weak::new());
}

/// Runs `f` on the app state during a later GLib idle. GTK signal handlers use this instead of
/// borrowing AppState inline, because a signal can fire while the state is already borrowed.
/// If the state is still borrowed when the idle runs, `f` is queued again.
pub fn defer(f: impl FnOnce(&mut AppState) + 'static) {
    glib::idle_add_local_once(move || {
        let Some(state) = APP_STATE.with(|slot| slot.borrow().upgrade()) else {
            return;
        };
        let busy = match state.try_borrow_mut() {
            Ok(mut s) => {
                f(&mut s);
                None
            }
            Err(_) => Some(f),
        };
        if let Some(f) = busy {
            defer(f);
        }
    });
}

/// Requests a debounced session save. Callable from ghostty callbacks; runs now if the state is free.
#[allow(dead_code)] // Called from the ghostty pwd callback (package T2).
pub fn request_session_save() {
    let Some(state) = APP_STATE.with(|slot| slot.borrow().upgrade()) else {
        return;
    };
    let busy = match state.try_borrow() {
        Ok(s) => {
            s.trigger_session_save();
            false
        }
        Err(_) => true,
    };
    if busy {
        defer(|s| s.trigger_session_save());
    }
}

/// Stack page name of a tab.
fn tab_page_name(tab_id: u64) -> String {
    format!("tab-{tab_id}")
}

/// A ghostty surface with one pane, started in `cwd` (the home folder when None).
fn create_engine(app: &gtk4::Application, ghostty_app: ffi::ghostty_app_t, cwd: Option<PathBuf>) -> SplitEngine {
    let pane_id = alloc_pane_id();
    let pane = crate::ghostty::surface::create_surface(
        app,
        ghostty_app,
        None,
        pane_id,
        crate::ghostty::surface::SurfaceIoMode::Exec,
        cwd,
    );
    SplitEngine::new(app.clone(), ghostty_app, pane, pane_id)
}

/// Sidebar row for a local workspace.
fn local_sidebar_row(name: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_child(Some(&crate::sidebar::rebuild_sidebar_row_content(name)));
    row
}

pub struct AppState {
    pub gtk_app: gtk4::Application,
    /// All open workspaces. Never empty after initialization — create_workspace is called in new().
    pub workspaces: Vec<Workspace>,
    /// Index into workspaces of the currently visible workspace.
    pub active_index: usize,
    /// GtkStack holding one page per workspace (the workspace's root GTK widget).
    pub stack: gtk4::Stack,
    /// GtkListBox in the sidebar showing workspace names.
    pub sidebar_list: gtk4::ListBox,
    /// Ghostty app handle — used by create_surface() for new panes.
    pub ghostty_app: ffi::ghostty_app_t,
    /// Next id for workspaces and tabs (monotonically increasing, shared).
    next_id: u64,
    /// Next display number for default names ("Workspace N").
    next_display_number: usize,
    /// Notified after any workspace/pane mutation to trigger a debounced session save.
    pub save_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// Sender for session snapshots to the debounce task.
    /// Each mutation snapshots SessionData on the main thread and sends it here.
    pub session_tx: Option<tokio::sync::mpsc::UnboundedSender<SessionData>>,
    /// Sender for SSH events (cloned into SSH lifecycle tokio tasks).
    pub ssh_event_tx: Option<crate::ssh::SshEventTx>,
    /// Tokio runtime handle for spawning SSH lifecycle tasks.
    pub runtime_handle: Option<tokio::runtime::Handle>,
    /// Handles to SSH lifecycle tasks, keyed by workspace id. Used for cleanup on close.
    pub ssh_task_handles: std::collections::HashMap<u64, tokio::task::JoinHandle<()>>,
    /// Maps pane_id -> IoWriteContext for remote panes (needed to set stream_id after proxy.open).
    pub remote_pane_contexts: std::collections::HashMap<u64, std::sync::Arc<crate::ssh::bridge::IoWriteContext>>,
    /// Maps workspace_id -> SshBridge for remote workspaces.
    pub workspace_bridges: std::collections::HashMap<u64, std::sync::Arc<crate::ssh::bridge::SshBridge>>,
    /// Browser preview daemon manager (Phase 8).
    pub browser_manager: Option<crate::browser::BrowserManager>,
    /// Next browser surface short-ref counter (monotonically increasing, per D-06).
    pub browser_surface_counter: u32,
    /// Maps short-ref ID -> surface UUID (lost on restart, per D-06).
    pub browser_surface_refs: std::collections::HashMap<u32, String>,
}

impl AppState {
    /// Create a new AppState. Does NOT create the first workspace — caller must call
    /// create_workspace() (or restore_session) after constructing the GTK widget tree.
    pub fn new(
        stack: gtk4::Stack,
        sidebar_list: gtk4::ListBox,
        ghostty_app: ffi::ghostty_app_t,
        gtk_app: gtk4::Application,
    ) -> AppStateRef {
        let state = AppState {
            workspaces: Vec::new(),
            active_index: 0,
            stack,
            sidebar_list,
            ghostty_app,
            gtk_app,
            next_id: 1,
            next_display_number: 1,
            save_notify: None, // Set to Some(...) after tokio runtime is available in main.rs
            session_tx: None,
            ssh_event_tx: None,
            runtime_handle: None,
            ssh_task_handles: std::collections::HashMap::new(),
            remote_pane_contexts: std::collections::HashMap::new(),
            workspace_bridges: std::collections::HashMap::new(),
            browser_manager: None,
            browser_surface_counter: 0,
            browser_surface_refs: std::collections::HashMap::new(),
        };
        let rc = Rc::new(RefCell::new(state));
        APP_STATE.with(|slot| *slot.borrow_mut() = Rc::downgrade(&rc));
        rc
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn alloc_display_number(&mut self) -> usize {
        let n = self.next_display_number;
        self.next_display_number += 1;
        n
    }

    fn ws_index(&self, ws_id: u64) -> Option<usize> {
        self.workspaces.iter().position(|ws| ws.id == ws_id)
    }

    /// Adds a workspace's sidebar row and its top-level stack page. Does not select it.
    fn append_workspace_page(&self, ws: &Workspace, row: gtk4::ListBoxRow) {
        unsafe {
            row.set_data("workspace-id", ws.id);
        }
        self.sidebar_list.append(&row);
        self.stack.add_named(&ws.ui.root, Some(&ws.stack_page_name));
    }

    /// Adds the tab's page to its workspace's tab stack. A lazy tab gets an empty placeholder
    /// until materialize_tab builds its split tree.
    fn attach_tab_page(ws: &Workspace, tab: &Tab) {
        let child: gtk4::Widget = match &tab.engine {
            Some(engine) => engine.root_widget(),
            None => gtk4::Box::new(gtk4::Orientation::Vertical, 0).upcast(),
        };
        ws.ui.tab_stack.add_named(&child, Some(&tab_page_name(tab.id)));
    }

    /// Builds the split tree of a lazy tab now that it is shown, replacing its placeholder page.
    /// Falls back to one fresh pane if the saved tree cannot be built. Does nothing for a live tab.
    fn materialize_tab(&mut self, wi: usize, ti: usize) {
        let app = self.gtk_app.clone();
        let ghostty = self.ghostty_app;
        let Some(ws) = self.workspaces.get_mut(wi) else {
            return;
        };
        let Some(tab) = ws.tabs.get_mut(ti) else {
            return;
        };
        if tab.engine.is_some() {
            return;
        }
        let engine = tab
            .saved
            .take()
            .and_then(|saved| {
                SplitEngine::from_data(app.clone(), ghostty, &saved.layout, saved.active_pane_uuid.as_deref())
            })
            .unwrap_or_else(|| create_engine(&app, ghostty, None));
        let page = tab_page_name(tab.id);
        if let Some(placeholder) = ws.ui.tab_stack.child_by_name(&page) {
            ws.ui.tab_stack.remove(&placeholder);
        }
        ws.ui.tab_stack.add_named(&engine.root_widget(), Some(&page));
        tab.engine = Some(engine);
    }

    /// Builds the active tab of the active workspace if it is still lazy.
    fn materialize_active(&mut self) {
        let wi = self.active_index;
        if let Some(ti) = self.workspaces.get(wi).map(|ws| ws.active_tab) {
            self.materialize_tab(wi, ti);
        }
    }

    /// Shows the workspace's selected tab in its tab stack.
    fn show_active_tab_page(&self, wi: usize) {
        if let Some(ws) = self.workspaces.get(wi) {
            if let Some(tab) = ws.tabs.get(ws.active_tab) {
                ws.ui.tab_stack.set_visible_child_name(&tab_page_name(tab.id));
            }
        }
    }

    /// Occlusion for the active tab. Only the visible tab of the visible workspace renders.
    fn set_active_visible(&self, visible: bool) {
        if let Some(engine) = self.active_split_engine() {
            engine.set_visible(visible);
        }
    }

    /// Redraws the tab bar of workspace `wi` from its tab list.
    fn refresh_tab_bar(&self, wi: usize) {
        let Some(ws) = self.workspaces.get(wi) else {
            return;
        };
        let tabs: Vec<(u64, String)> = ws.tabs.iter().map(|t| (t.id, t.name.clone())).collect();
        let active = ws.tabs.get(ws.active_tab).map(|t| t.id).unwrap_or(0);
        ws.ui.tab_bar.set_tabs(&tabs, active);
    }

    /// Recomputes the workspace's attention flag and its sidebar dot.
    fn refresh_attention(&mut self, wi: usize) {
        if let Some(ws) = self.workspaces.get_mut(wi) {
            ws.has_attention = ws.any_attention();
        }
        self.update_sidebar_attention(wi);
    }

    /// Creates a workspace with one tab in the focused pane's folder, and selects it.
    pub fn new_workspace(&mut self) -> u64 {
        let cwd = self.active_split_engine().and_then(|e| e.focused_cwd());
        let ws_id = self.alloc_id();
        let display_number = self.alloc_display_number();
        let mut workspace = Workspace::new(ws_id, display_number, WorkspaceUi::new(ws_id));
        let tab_id = self.alloc_id();
        let name = workspace.take_default_tab_name();
        let tab = Tab::live(tab_id, name, self.new_engine(cwd));
        Self::attach_tab_page(&workspace, &tab);
        workspace.tabs.push(tab);

        let row = local_sidebar_row(&workspace.name);
        self.append_workspace_page(&workspace, row);
        self.workspaces.push(workspace);
        let new_index = self.workspaces.len() - 1;
        self.refresh_tab_bar(new_index);
        self.switch_to_index(new_index);
        self.trigger_session_save();
        ws_id
    }

    /// Same as new_workspace. Kept for the socket and shortcut callers.
    pub fn create_workspace(&mut self) -> u64 {
        self.new_workspace()
    }

    /// Restores one workspace from its saved tabs. Every tab stays lazy: no shell starts until the
    /// tab is shown. Returns the workspace id, or None if the workspace has no tabs.
    pub fn restore_workspace(&mut self, ws_session: &WorkspaceSession) -> Option<u64> {
        if ws_session.tabs.is_empty() {
            return None;
        }
        let ws_id = self.alloc_id();
        let display_number = self.alloc_display_number();
        let mut workspace = Workspace::new(ws_id, display_number, WorkspaceUi::new(ws_id));
        if let Ok(uuid) = Uuid::parse_str(&ws_session.id) {
            workspace.uuid = uuid;
        }
        workspace.name = ws_session.name.clone();

        for saved in &ws_session.tabs {
            let tab_id = self.alloc_id();
            let name = if saved.name.trim().is_empty() {
                workspace.take_default_tab_name()
            } else {
                saved.name.clone()
            };
            let tab = Tab::lazy(
                tab_id,
                name,
                SavedLayout {
                    layout: saved.layout.clone(),
                    active_pane_uuid: saved.active_pane_uuid.clone(),
                },
            );
            Self::attach_tab_page(&workspace, &tab);
            workspace.tabs.push(tab);
        }
        workspace.active_tab = ws_session.selected_tab.min(workspace.tabs.len() - 1);

        let row = local_sidebar_row(&workspace.name);
        self.append_workspace_page(&workspace, row);
        self.workspaces.push(workspace);
        let wi = self.workspaces.len() - 1;
        self.refresh_tab_bar(wi);
        Some(ws_id)
    }

    /// Restores every workspace in `session` (lazily) and selects the saved one. Returns how many
    /// workspaces were restored. 0 means the caller should create a default workspace.
    pub fn restore_session(&mut self, session: &SessionData) -> usize {
        let mut restored = 0;
        let mut selected = None;
        for (i, ws_session) in session.workspaces.iter().enumerate() {
            if self.restore_workspace(ws_session).is_some() {
                restored += 1;
                if i == session.selected_workspace {
                    selected = Some(self.workspaces.len() - 1);
                }
            }
        }
        if restored > 0 {
            self.switch_to_index(selected.unwrap_or(0));
        }
        restored
    }

    /// Build a sidebar row for a workspace. Used by create_remote_workspace.
    fn build_sidebar_row(&self, workspace: &Workspace) -> gtk4::ListBoxRow {
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let label = gtk4::Label::new(Some(&workspace.name));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        vbox.append(&label);

        // SSH connection state subtitle (only for remote workspaces)
        if workspace.connection_state.is_remote() {
            let status = gtk4::Label::new(Some(workspace.connection_state.display_text()));
            status.set_halign(gtk4::Align::Start);
            status.add_css_class("connection-state");
            status.add_css_class(workspace.connection_state.css_class());
            vbox.append(&status);
        }
        vbox.set_hexpand(true);
        hbox.append(&vbox);

        // Attention dot — hidden by default, shown when has_attention
        let dot = gtk4::Label::new(None);
        dot.add_css_class("attention-dot");
        dot.set_visible(false);
        hbox.append(&dot);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&hbox));
        row
    }

    /// Create a remote SSH workspace. Returns workspace id.
    /// The bridge is used to create an IoWriteContext for the initial pane's manual I/O mode surface.
    pub fn create_remote_workspace(
        &mut self,
        target: String,
        bridge: &std::sync::Arc<crate::ssh::bridge::SshBridge>,
    ) -> u64 {
        let ws_id = self.alloc_id();
        let display_number = self.alloc_display_number();
        let mut workspace = Workspace::new_remote(ws_id, display_number, target, WorkspaceUi::new(ws_id));

        // Create remote surface with manual I/O mode
        let pane_id = alloc_pane_id();
        let io_ctx = std::sync::Arc::new(crate::ssh::bridge::IoWriteContext {
            pane_id,
            write_tx: bridge.clone_write_tx(),
            stream_id: std::sync::Mutex::new(None),
            eof_received: std::sync::atomic::AtomicBool::new(false),
            ssh_tx: self.ssh_event_tx.clone().expect("ssh_event_tx must be set before creating remote workspaces"),
        });
        let pane = crate::ghostty::surface::create_surface(
            &self.gtk_app,
            self.ghostty_app,
            None,
            pane_id,
            crate::ghostty::surface::SurfaceIoMode::Manual { io_write_ctx: io_ctx.clone() },
            None,
        );
        let engine = SplitEngine::new(self.gtk_app.clone(), self.ghostty_app, pane, pane_id);
        let tab_id = self.alloc_id();
        let name = workspace.take_default_tab_name();
        let tab = Tab::live(tab_id, name, engine);
        Self::attach_tab_page(&workspace, &tab);
        workspace.tabs.push(tab);

        let row = self.build_sidebar_row(&workspace);
        self.append_workspace_page(&workspace, row);
        self.workspaces.push(workspace);

        // Register pane in bridge.streams so run_proxy_routing can find it
        // and open a remote stream after SSH handshake completes.
        bridge.register_pane_placeholder(pane_id);

        // Store IoWriteContext for stream_id wiring when StreamOpened arrives
        self.remote_pane_contexts.insert(pane_id, io_ctx);

        let wi = self.workspaces.len() - 1;
        self.refresh_tab_bar(wi);
        self.switch_to_index(wi);
        self.trigger_session_save();
        ws_id
    }

    /// Update the connection state of a workspace and refresh its sidebar row.
    pub fn update_connection_state(&mut self, workspace_id: u64, state: ConnectionState) {
        if let Some(idx) = self.ws_index(workspace_id) {
            self.workspaces[idx].connection_state = state.clone();
            // Update sidebar subtitle
            if let Some(row) = self.sidebar_list.row_at_index(idx as i32) {
                if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                    if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                        // Last child in vbox is the status label (if it has connection-state class)
                        if let Some(status) = vbox.last_child().and_downcast::<gtk4::Label>() {
                            if status.has_css_class("connection-state") {
                                status.set_text(state.display_text());
                                status.remove_css_class("connected");
                                status.remove_css_class("disconnected");
                                status.remove_css_class("reconnecting");
                                status.add_css_class(state.css_class());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Close the workspace at `index`. Tears down the panes of all its tabs, then removes the
    /// sidebar row and GtkStack page. Returns false if there is only one workspace.
    pub fn close_workspace(&mut self, index: usize) -> bool {
        if self.workspaces.len() <= 1 || index >= self.workspaces.len() {
            return false; // Cannot close the last workspace
        }

        // Abort SSH lifecycle task if this is a remote workspace.
        if let Some(ws) = self.workspaces.get(index) {
            if let Some(handle) = self.ssh_task_handles.remove(&ws.id) {
                handle.abort();
            }
        }

        // Hide the panes before they are torn down.
        if index == self.active_index {
            self.set_active_visible(false);
        }
        // Tear down every built pane's surface and GL state before any widget is removed.
        for tab in &self.workspaces[index].tabs {
            if let Some(engine) = &tab.engine {
                engine.teardown_all();
            }
        }

        let workspace = self.workspaces.remove(index);

        // Remove sidebar row.
        if let Some(row) = self.sidebar_list.row_at_index(index as i32) {
            self.sidebar_list.remove(&row);
        }

        // Remove GtkStack page.
        if let Some(child) = self.stack.child_by_name(&workspace.stack_page_name) {
            self.stack.remove(&child);
        }

        // Adjust active_index: if we removed before or at active, clamp.
        if self.active_index >= self.workspaces.len() {
            self.active_index = self.workspaces.len() - 1;
        } else if index <= self.active_index && self.active_index > 0 {
            self.active_index -= 1;
        }

        self.switch_to_index(self.active_index);
        self.trigger_session_save();
        true
    }

    /// Moves workspace `from` to position `to`, keeping its sidebar row and the selection.
    pub fn move_workspace(&mut self, from: usize, to: usize) {
        let len = self.workspaces.len();
        if from >= len || len == 0 {
            return;
        }
        let to = to.min(len - 1);
        if from == to {
            return;
        }
        let ws = self.workspaces.remove(from);
        self.workspaces.insert(to, ws);
        if let Some(row) = self.sidebar_list.row_at_index(from as i32) {
            self.sidebar_list.remove(&row);
            self.sidebar_list.insert(&row, to as i32);
        }
        self.active_index = workspace::active_after_move(self.active_index, from, to);
        self.switch_to_index(self.active_index);
        self.trigger_session_save();
    }

    /// The workspace and tab index holding `pane_id`, if that pane is built.
    fn find_pane(&self, pane_id: u64) -> Option<(usize, usize)> {
        self.workspaces.iter().enumerate().find_map(|(wi, ws)| {
            ws.tabs.iter().enumerate().find_map(|(ti, tab)| {
                tab.engine
                    .as_ref()
                    .filter(|e| e.contains_pane(pane_id))
                    .map(|_| (wi, ti))
            })
        })
    }

    /// Close one pane, e.g. because its shell exited. The last pane of a tab closes the tab, and
    /// the last tab of a workspace is replaced by a fresh one. Returns true if the pane was closed.
    pub fn close_pane(&mut self, pane_id: u64) -> bool {
        let Some((wi, ti)) = self.find_pane(pane_id) else {
            return false;
        };
        let result = match self.workspaces[wi].tabs[ti].engine.as_mut() {
            Some(engine) => engine.close_pane(pane_id),
            None => return false,
        };
        match result {
            CloseResult::Closed(_) => {
                self.trigger_session_save();
                true
            }
            CloseResult::LastPane => self.close_tab_at(wi, ti),
            CloseResult::Failed => false,
        }
    }

}

// Tab API (Workspace -> Tabs -> Splits). Indexes refer to the active workspace unless the name
// says otherwise. Shortcuts and the socket reach these through the keymap package; until it is
// wired, the unused ones are allowed here.
#[allow(dead_code)]
impl AppState {

    fn new_engine(&self, cwd: Option<PathBuf>) -> SplitEngine {
        create_engine(&self.gtk_app, self.ghostty_app, cwd)
    }

    /// Opens a new tab in the focused pane's folder, appends it and selects it.
    /// Returns None for a remote workspace (its panes need the SSH bridge, which tabs do not have yet).
    pub fn new_tab(&mut self) -> Option<u64> {
        let wi = self.active_index;
        let ws = self.workspaces.get(wi)?;
        if ws.remote_target.is_some() {
            eprintln!("cmux: new tabs in SSH workspaces are not supported yet");
            return None;
        }
        let cwd = ws.active_engine().and_then(|e| e.focused_cwd());
        let tab_id = self.alloc_id();
        let name = self.workspaces[wi].take_default_tab_name();
        let engine = self.new_engine(cwd);

        self.set_active_visible(false);
        let ws = &mut self.workspaces[wi];
        let tab = Tab::live(tab_id, name, engine);
        Self::attach_tab_page(ws, &tab);
        ws.tabs.push(tab);
        ws.active_tab = ws.tabs.len() - 1;
        self.show_active_tab_page(wi);
        self.set_active_visible(true);
        if let Some(engine) = self.active_split_engine() {
            engine.focus_active_surface();
        }
        self.refresh_tab_bar(wi);
        self.trigger_session_save();
        Some(tab_id)
    }

    /// Opens a tab in the same folder as the active tab, named like it.
    pub fn duplicate_tab(&mut self) -> Option<u64> {
        let ws = self.workspaces.get(self.active_index)?;
        let name = ws.tabs.get(ws.active_tab)?.name.clone();
        let id = self.new_tab()?;
        self.rename_active_tab(name);
        Some(id)
    }

    /// Selects tab `ti` of workspace `wi`. Builds the tab if it is still lazy, and moves the
    /// occlusion to the newly visible tab when `wi` is the active workspace.
    fn select_tab_in(&mut self, wi: usize, ti: usize) {
        let Some(ws) = self.workspaces.get(wi) else {
            return;
        };
        if ti >= ws.tabs.len() {
            return;
        }
        let is_active_ws = wi == self.active_index;
        if is_active_ws && ws.active_tab != ti {
            self.set_active_visible(false);
        }
        self.workspaces[wi].active_tab = ti;
        self.materialize_tab(wi, ti);
        self.show_active_tab_page(wi);
        if let Some(engine) = self.workspaces[wi].tabs[ti].engine.as_mut() {
            engine.root.clear_all_attention();
        }
        self.refresh_attention(wi);
        if is_active_ws {
            self.set_active_visible(true);
            if let Some(engine) = self.active_split_engine() {
                engine.focus_active_surface();
            }
        }
        self.refresh_tab_bar(wi);
        self.trigger_session_save();
    }

    /// Selects tab `idx` of the active workspace.
    pub fn select_tab(&mut self, idx: usize) {
        let wi = self.active_index;
        self.select_tab_in(wi, idx);
    }

    /// Next tab of the active workspace, wrapping around.
    pub fn next_tab(&mut self) {
        let Some(ws) = self.workspaces.get(self.active_index) else {
            return;
        };
        let to = workspace::next_tab_index(ws.tabs.len(), ws.active_tab);
        self.select_tab(to);
    }

    /// Previous tab of the active workspace, wrapping around.
    pub fn prev_tab(&mut self) {
        let Some(ws) = self.workspaces.get(self.active_index) else {
            return;
        };
        let to = workspace::prev_tab_index(ws.tabs.len(), ws.active_tab);
        self.select_tab(to);
    }

    /// Selects the last tab of the active workspace.
    pub fn select_last_tab(&mut self) {
        let Some(len) = self.workspaces.get(self.active_index).map(|ws| ws.tabs.len()) else {
            return;
        };
        if len > 0 {
            self.select_tab(len - 1);
        }
    }

    /// Moves tab `from` to position `to` in the active workspace. `to` is clamped to the last tab.
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        let wi = self.active_index;
        self.move_tab_in(wi, from, to)
    }

    fn move_tab_in(&mut self, wi: usize, from: usize, to: usize) -> bool {
        let Some(ws) = self.workspaces.get_mut(wi) else {
            return false;
        };
        let len = ws.tabs.len();
        let to = to.min(len.saturating_sub(1));
        if from >= len || from == to {
            return false;
        }
        let tab = ws.tabs.remove(from);
        ws.tabs.insert(to, tab);
        ws.active_tab = workspace::active_after_move(ws.active_tab, from, to);
        self.refresh_tab_bar(wi);
        self.trigger_session_save();
        true
    }

    /// Moves the tab `from_id` to the position of the tab `to_id` (drag and drop in the tab bar).
    pub fn move_tab_by_id(&mut self, ws_id: u64, from_id: u64, to_id: u64) {
        let Some(wi) = self.ws_index(ws_id) else {
            return;
        };
        let ws = &self.workspaces[wi];
        if let (Some(from), Some(to)) = (ws.tab_index(from_id), ws.tab_index(to_id)) {
            self.move_tab_in(wi, from, to);
        }
    }

    /// Sets the name of tab `ti` of workspace `wi`. Blank names are ignored. Returns true if renamed.
    fn set_tab_name(&mut self, wi: usize, ti: usize, name: &str) -> bool {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return false;
        }
        match self.workspaces.get_mut(wi).and_then(|ws| ws.tabs.get_mut(ti)) {
            Some(tab) => {
                tab.name = trimmed.to_string();
                true
            }
            None => false,
        }
    }

    /// Renames the selected tab of the active workspace.
    pub fn rename_active_tab(&mut self, name: String) {
        let wi = self.active_index;
        let Some(ti) = self.workspaces.get(wi).map(|ws| ws.active_tab) else {
            return;
        };
        if self.set_tab_name(wi, ti, &name) {
            self.refresh_tab_bar(wi);
            self.trigger_session_save();
        }
    }

    /// Starts inline renaming of a tab (double click in the tab bar).
    pub fn begin_tab_rename(&mut self, ws_id: u64, tab_id: u64) {
        let Some(wi) = self.ws_index(ws_id) else {
            return;
        };
        if self.workspaces[wi].tab_index(tab_id).is_none() {
            return;
        }
        // A second click on the open editor must not rebuild it.
        if self.workspaces[wi].ui.tab_bar.renaming() == Some(tab_id) {
            return;
        }
        self.workspaces[wi].ui.tab_bar.set_renaming(Some(tab_id));
        self.refresh_tab_bar(wi);
    }

    /// Ends inline renaming. `Some(text)` commits, `None` cancels. Only the first call for an
    /// editor has an effect, so Enter followed by focus-out commits once.
    pub fn end_tab_rename(&mut self, ws_id: u64, tab_id: u64, text: Option<String>) {
        let Some(wi) = self.ws_index(ws_id) else {
            return;
        };
        if self.workspaces[wi].ui.tab_bar.renaming() != Some(tab_id) {
            return;
        }
        self.workspaces[wi].ui.tab_bar.set_renaming(None);
        if let (Some(text), Some(ti)) = (text, self.workspaces[wi].tab_index(tab_id)) {
            self.set_tab_name(wi, ti, &text);
        }
        self.refresh_tab_bar(wi);
        self.trigger_session_save();
    }

    /// Selects a tab by id, switching to its workspace first if needed.
    pub fn select_tab_by_id(&mut self, ws_id: u64, tab_id: u64) {
        let Some(wi) = self.ws_index(ws_id) else {
            return;
        };
        let Some(ti) = self.workspaces[wi].tab_index(tab_id) else {
            return;
        };
        if wi != self.active_index {
            self.switch_to_index(wi);
        }
        self.select_tab_in(wi, ti);
    }

    /// Closes a tab by id.
    pub fn close_tab_by_id(&mut self, ws_id: u64, tab_id: u64) -> bool {
        let Some(wi) = self.ws_index(ws_id) else {
            return false;
        };
        let Some(ti) = self.workspaces[wi].tab_index(tab_id) else {
            return false;
        };
        self.close_tab_at(wi, ti)
    }

    /// Closes tab `idx` of the active workspace.
    pub fn close_tab(&mut self, idx: usize) -> bool {
        let wi = self.active_index;
        self.close_tab_at(wi, idx)
    }

    /// Closes the selected tab of the active workspace.
    pub fn close_active_tab(&mut self) -> bool {
        let wi = self.active_index;
        let Some(ti) = self.workspaces.get(wi).map(|ws| ws.active_tab) else {
            return false;
        };
        self.close_tab_at(wi, ti)
    }

    /// Closes tab `ti` of workspace `wi`. Its panes are torn down before the widget is removed.
    /// Closing a workspace's last tab opens a fresh tab in the same folder first, so a workspace
    /// never has zero tabs.
    pub fn close_tab_at(&mut self, wi: usize, ti: usize) -> bool {
        let Some(ws) = self.workspaces.get(wi) else {
            return false;
        };
        if ti >= ws.tabs.len() {
            return false;
        }
        // The last tab of an SSH workspace closes the workspace rather than opening a local shell in it.
        if ws.tabs.len() == 1 && ws.remote_target.is_some() && self.workspaces.len() > 1 {
            return self.close_workspace(wi);
        }
        if ws.tabs.len() == 1 {
            let cwd = ws.tabs[0].cwd();
            let tab_id = self.alloc_id();
            let name = self.workspaces[wi].take_default_tab_name();
            let tab = Tab::live(tab_id, name, self.new_engine(cwd));
            let ws = &mut self.workspaces[wi];
            Self::attach_tab_page(ws, &tab);
            ws.tabs.push(tab);
        }

        let is_active_ws = wi == self.active_index;
        let ws = &self.workspaces[wi];
        let new_active = workspace::active_after_close(ws.tabs.len(), ws.active_tab, ti);
        let closing_active = ws.active_tab == ti;
        if is_active_ws && closing_active {
            self.set_active_visible(false);
        }

        let ws = &mut self.workspaces[wi];
        let tab = ws.tabs.remove(ti);
        if let Some(engine) = &tab.engine {
            engine.teardown_all();
        }
        if let Some(child) = ws.ui.tab_stack.child_by_name(&tab_page_name(tab.id)) {
            ws.ui.tab_stack.remove(&child);
        }
        drop(tab);
        ws.active_tab = new_active;

        if is_active_ws && closing_active {
            // The neighbour may still be lazy: build it now that it is shown.
            self.materialize_tab(wi, new_active);
            self.show_active_tab_page(wi);
            self.set_active_visible(true);
            if let Some(engine) = self.active_split_engine() {
                engine.focus_active_surface();
            }
        } else {
            self.show_active_tab_page(wi);
        }
        self.refresh_attention(wi);
        self.refresh_tab_bar(wi);
        self.trigger_session_save();
        true
    }

}

impl AppState {
    /// Switch to the workspace at `index` (0-based). Updates GtkStack visible child and
    /// sidebar selection. Does nothing if index is out of bounds. Panes of the workspace being
    /// left are occluded, and the selected tab of the new workspace is built if it is lazy.
    pub fn switch_to_index(&mut self, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        if index != self.active_index {
            self.set_active_visible(false);
        }
        // Phase 4: clear attention when user switches to a workspace (D-05).
        self.clear_workspace_attention(index);
        self.active_index = index;
        let page_name = self.workspaces[index].stack_page_name.clone();
        self.stack.set_visible_child_name(&page_name);
        self.materialize_active();
        self.show_active_tab_page(index);
        if let Some(row) = self.sidebar_list.row_at_index(index as i32) {
            self.sidebar_list.select_row(Some(&row));
            // Update CSS classes: active row gets "active-workspace" for styling.
            // All rows: remove first, then add to active.
            let count = self.workspaces.len() as i32;
            for i in 0..count {
                if let Some(r) = self.sidebar_list.row_at_index(i) {
                    r.remove_css_class("active-workspace");
                    // Phase 4: navigate nested layout: row > hbox > vbox > label
                    if let Some(hbox) = r.child().and_downcast::<gtk4::Box>() {
                        if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                            if let Some(label) = vbox.first_child().and_downcast::<gtk4::Label>() {
                                label.set_css_classes(&[]);
                            }
                        }
                    }
                }
            }
            row.add_css_class("active-workspace");
            // Phase 4: navigate nested layout: row > hbox > vbox > label
            if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                    if let Some(label) = vbox.first_child().and_downcast::<gtk4::Label>() {
                        label.add_css_class("active-workspace-label");
                    }
                }
            }
        }
        self.set_active_visible(true);
        // Grab GTK keyboard focus on the active pane so key events reach Ghostty.
        if let Some(engine) = self.active_split_engine() {
            engine.grab_active_focus();
        }
    }

    /// Selects the workspace at `idx`. Same as switch_to_index.
    pub fn select_workspace(&mut self, idx: usize) {
        self.switch_to_index(idx);
    }

    /// Switch to next workspace (wrap-around). Per D-10: Ctrl+].
    pub fn switch_next(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        let next = (self.active_index + 1) % self.workspaces.len();
        self.switch_to_index(next);
    }

    /// Switch to previous workspace (wrap-around). Per D-10: Ctrl+[.
    pub fn switch_prev(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        let prev = if self.active_index == 0 {
            self.workspaces.len() - 1
        } else {
            self.active_index - 1
        };
        self.switch_to_index(prev);
    }

    /// Split tree of the active tab of the active workspace. Callers that split, close, or focus
    /// panes use this, so they act on the visible tab.
    pub fn active_split_engine(&self) -> Option<&SplitEngine> {
        self.workspaces.get(self.active_index)?.active_engine()
    }

    pub fn active_split_engine_mut(&mut self) -> Option<&mut SplitEngine> {
        self.workspaces.get_mut(self.active_index)?.active_engine_mut()
    }

    /// Rename the active workspace. Per D-03/D-10: Ctrl+Shift+R (UI wired in Plan 04/05).
    pub fn rename_active(&mut self, new_name: String) {
        if let Some(ws) = self.workspaces.get_mut(self.active_index) {
            ws.rename(new_name.clone());
            // Update the sidebar label (Phase 4 nested layout: row > hbox > vbox > label).
            if let Some(row) = self.sidebar_list.row_at_index(self.active_index as i32) {
                if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                    if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                        if let Some(label) = vbox.first_child().and_downcast::<gtk4::Label>() {
                            label.set_text(&new_name);
                        }
                    }
                }
            }
            self.trigger_session_save();
        }
    }

    /// Returns the active workspace, if any.
    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.active_index)
    }

    /// Set attention on a specific pane. Called from bell handler.
    /// Updates workspace has_attention and sidebar dot.
    pub fn set_pane_attention(&mut self, pane_id: u64) {
        let mut found = None;
        'search: for (wi, ws) in self.workspaces.iter_mut().enumerate() {
            for tab in ws.tabs.iter_mut() {
                if let Some(engine) = tab.engine.as_mut() {
                    if engine.root.set_attention(pane_id, true) {
                        found = Some(wi);
                        break 'search;
                    }
                }
            }
        }
        let Some(idx) = found else {
            return;
        };
        self.refresh_attention(idx);

        // Desktop notification when window is unfocused (NOTF-03)
        let window_focused = self.gtk_app.active_window()
            .map(|w| w.is_active())
            .unwrap_or(false);
        if !window_focused && self.workspaces[idx].has_attention {
            let should_notify = self.workspaces[idx].last_notification
                .map(|t| t.elapsed() >= std::time::Duration::from_secs(5))
                .unwrap_or(true);
            if should_notify {
                self.workspaces[idx].last_notification = Some(std::time::Instant::now());
                send_bell_notification(&self.gtk_app, &self.workspaces[idx].name, idx);
            }
        }
    }

    /// Clear all attention in the workspace at `index`, in every tab.
    pub fn clear_workspace_attention(&mut self, index: usize) {
        if let Some(ws) = self.workspaces.get_mut(index) {
            for tab in ws.tabs.iter_mut() {
                if let Some(engine) = tab.engine.as_mut() {
                    engine.root.clear_all_attention();
                }
            }
            ws.has_attention = false;
        }
        self.update_sidebar_attention(index);
    }

    /// Update the sidebar dot visibility for workspace at `index`.
    fn update_sidebar_attention(&self, index: usize) {
        if let Some(row) = self.sidebar_list.row_at_index(index as i32) {
            let has_attention = self.workspaces.get(index)
                .map(|ws| ws.has_attention)
                .unwrap_or(false);
            // Row layout: GtkBox(H) > [GtkBox(V) > [GtkLabel(name)], GtkLabel(dot)]
            if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                if let Some(dot) = hbox.last_child() {
                    dot.set_visible(has_attention);
                }
            }
        }
    }

    /// Shut down the agent-browser daemon if running (called on app exit).
    pub fn shutdown_browser(&mut self) {
        if let Some(ref mut bm) = self.browser_manager {
            eprintln!("cmux: shutting down browser daemon");
            bm.shutdown();
            self.browser_manager = None;
        }
    }

    /// Session snapshot of every workspace and tab. A lazy tab keeps its saved layout, so
    /// restoring and saving again loses nothing. Tabs and workspaces with no terminal are skipped.
    pub fn snapshot_session(&self) -> SessionData {
        let mut workspaces = Vec::new();
        let mut selected_workspace = 0;
        for (wi, ws) in self.workspaces.iter().enumerate() {
            let mut tabs = Vec::new();
            let mut selected_tab = 0;
            for (ti, tab) in ws.tabs.iter().enumerate() {
                let Some(entry) = tab.session_entry() else {
                    continue;
                };
                if ti == ws.active_tab {
                    selected_tab = tabs.len();
                }
                tabs.push(entry);
            }
            if tabs.is_empty() {
                continue;
            }
            if wi == self.active_index {
                selected_workspace = workspaces.len();
            }
            workspaces.push(WorkspaceSession {
                id: ws.uuid.to_string(),
                name: ws.name.clone(),
                selected_tab,
                tabs,
            });
        }
        SessionData {
            version: SESSION_VERSION,
            selected_workspace,
            workspaces,
        }
    }

    /// Trigger a debounced session save. Call after any workspace/tab/pane mutation.
    /// Snapshots SessionData on the main thread (safe for Rc) and sends to the
    /// tokio debounce task which handles the file I/O.
    pub fn trigger_session_save(&self) {
        if let (Some(notify), Some(tx)) = (&self.save_notify, &self.session_tx) {
            let _ = tx.send(self.snapshot_session());
            notify.notify_one();
        }
    }

    /// Writes the session now, on this thread. Used on quit and on SIGTERM/SIGHUP, where the
    /// debounced task may not get to run.
    pub fn flush_session(&self) {
        if self.save_notify.is_none() {
            return;
        }
        if let Err(e) = session::save_session_atomic(&self.snapshot_session()) {
            cmux_warn!("session flush failed: {e}");
        }
    }
}

/// Closes a pane whose surface ghostty asked to close (the shell exited).
/// Called from an idle callback on the GLib main thread, never from inside ghostty_app_tick.
/// Does nothing if the app state is busy or the pane is already gone.
pub fn close_pane_request(pane_id: u64) {
    let Some(state) = APP_STATE.with(|slot| slot.borrow().upgrade()) else {
        return;
    };
    let Ok(mut s) = state.try_borrow_mut() else {
        // Something holds the state right now; try again on a later idle.
        glib::idle_add_local_once(move || close_pane_request(pane_id));
        return;
    };
    s.close_pane(pane_id);
}

/// Send a desktop notification for a bell in the given workspace.
/// Uses `notify-send` subprocess to send notifications via org.freedesktop.Notifications.
///
/// We use a subprocess instead of notify-rust (zbus D-Bus client) because GNOME Shell
/// destroys notifications when the D-Bus sender name vanishes. With notify-rust in a
/// spawned thread, the zbus connection drops when the thread exits, causing GNOME Shell's
/// FdoNotificationDaemonSource._onNameVanished() to destroy the notification immediately.
/// `notify-send` avoids this because it's a separate process whose D-Bus lifetime is
/// independent of cmux.
fn send_bell_notification(_app: &gtk4::Application, workspace_name: &str, _workspace_index: usize) {
    let body = format!("{} - Terminal bell", workspace_name);
    std::thread::spawn(move || {
        let result = std::process::Command::new("notify-send")
            .arg("--app-name=cmux")
            .arg("--icon=utilities-terminal")
            .arg("--expire-time=5000")
            .arg("Terminal Bell")
            .arg(&body)
            .status();
        match result {
            Ok(status) if !status.success() => {
                eprintln!("cmux: notify-send exited with {status}");
            }
            Err(e) => {
                eprintln!("cmux: failed to run notify-send: {e}");
            }
            _ => {}
        }
    });
}
