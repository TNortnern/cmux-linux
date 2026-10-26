use crate::app_state::{defer, with_state, AppStateRef};
use gtk4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Key under which a sidebar row keeps the id of the workspace it shows.
const ROW_WORKSPACE_KEY: &str = "workspace-id";

/// Build the sidebar widget: outer Box(V) > [ScrolledWindow(ListBox), Button(+)].
/// Returns (sidebar_box, scrolled_window, list_box).
///
/// Per Pitfall 5 from RESEARCH.md: the '+' button is OUTSIDE the ScrolledWindow
/// so it doesn't scroll away.
///
/// Per UI-SPEC:
/// - Width: 160px (set_size_request(160, -1))
/// - Background: #242424 (applied via global CssProvider in main.rs)
/// - Row height: 36px min-height (CSS)
/// - Row padding: 8px top/bottom, 16px left/right
/// - Active row: #5b8dd9 background, #ffffff text, font-weight 600
/// - Inactive row: transparent bg, #cccccc text, font-weight 400
/// - Hover (inactive): #2e2e2e
pub fn build_sidebar() -> (gtk4::Box, gtk4::ScrolledWindow, gtk4::ListBox) {
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("workspace-list");

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_size_request(160, -1);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scrolled.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    scrolled.set_child(Some(&list_box));
    scrolled.set_vexpand(true);

    // Sidebar container: Box(V) > [ScrolledWindow(ListBox), Button(+)]
    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");
    sidebar_box.append(&scrolled);

    // '+' button at the bottom (D-01)
    let add_btn = gtk4::Button::with_label("+");
    add_btn.add_css_class("sidebar-add-btn");
    add_btn.set_tooltip_text(Some("New Workspace (Ctrl+N)"));
    add_btn.set_action_name(Some("win.new-workspace"));
    sidebar_box.append(&add_btn);

    (sidebar_box, scrolled, list_box)
}

/// Wire sidebar click-to-switch. Called from main.rs after AppState is constructed.
/// Per WS-03: clicking a row selects its workspace. The change runs through `defer`, since the
/// activation can fire while AppState is borrowed.
pub fn wire_sidebar_clicks(list_box: &gtk4::ListBox) {
    list_box.connect_row_activated(|_list, row| {
        let Some(ws_id) = row_workspace_id(row) else {
            return;
        };
        defer(move |s| s.select_workspace_by_id(ws_id));
    });
}

/// Records which workspace a sidebar row shows.
pub fn set_row_workspace_id(row: &gtk4::ListBoxRow, ws_id: u64) {
    // SAFETY: this key is only ever written with a u64, by this function.
    unsafe {
        row.set_data(ROW_WORKSPACE_KEY, ws_id);
    }
}

/// The workspace a sidebar row shows, if it was recorded.
pub fn row_workspace_id(row: &gtk4::ListBoxRow) -> Option<u64> {
    // SAFETY: the only value stored under this key is a u64 (see set_row_workspace_id).
    unsafe { row.data::<u64>(ROW_WORKSPACE_KEY).map(|p| *p.as_ref()) }
}

/// The name label of a row's content box: hbox > vbox > label.
fn name_label_of(hbox: &gtk4::Box) -> Option<gtk4::Label> {
    hbox.first_child()
        .and_downcast::<gtk4::Box>()?
        .first_child()
        .and_downcast::<gtk4::Label>()
}

/// Shows `name` on a row. Does nothing while the row is being renamed (its child is an entry).
pub fn set_row_name(row: &gtk4::ListBoxRow, name: &str) {
    if let Some(label) = row.child().and_downcast::<gtk4::Box>().and_then(|h| name_label_of(&h)) {
        label.set_text(name);
    }
}

/// The attention dot of a row's content box: the child with the `attention-dot` class.
pub fn attention_dot_of(row: &gtk4::ListBoxRow) -> Option<gtk4::Widget> {
    let hbox = row.child().and_downcast::<gtk4::Box>()?;
    let mut child = hbox.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("attention-dot") {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

/// Start inline rename for the active workspace row.
/// Per UI-SPEC: replaces the name with a GtkEntry; Enter commits, Escape cancels.
/// Per D-03: rename triggered by Ctrl+Shift+R (keyboard only).
///
/// The row's own content box is kept and put back afterwards, so its close button and status
/// line survive the rename. The commit is queued through `defer`, and it renames the workspace
/// that was active when the rename started.
pub fn start_inline_rename(list_box: &gtk4::ListBox, state: &AppStateRef) {
    let (index, ws_id) = {
        let s = state.borrow();
        (s.active_index, s.active_workspace().map(|ws| ws.id))
    };
    let (Some(ws_id), Some(row)) = (ws_id, list_box.row_at_index(index as i32)) else {
        return;
    };
    // A row already being renamed has an entry as its child, not the content box.
    let Some(content) = row.child().and_downcast::<gtk4::Box>() else {
        return;
    };
    let Some(name_label) = name_label_of(&content) else {
        return;
    };
    let current_name = name_label.text().to_string();

    let entry = gtk4::Entry::new();
    entry.set_text(&current_name);
    entry.set_placeholder_text(Some("Workspace name"));
    entry.add_css_class("rename-entry");
    // Setting the entry as the child unparents `content`; the handle kept here puts it back.
    row.set_child(Some(&entry));

    // Ends the rename once: Enter, focus-out and Escape can all fire for the same edit.
    let done = Rc::new(Cell::new(false));
    let finish: Rc<dyn Fn(Option<String>)> = Rc::new({
        let done = done.clone();
        let row = row.clone();
        let content = content.clone();
        move |text: Option<String>| {
            if done.replace(true) {
                return;
            }
            let name = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            if let Some(label) = name_label_of(&content) {
                label.set_text(name.as_deref().unwrap_or(&current_name));
            }
            row.set_child(Some(&content));
            if let Some(name) = name {
                defer(move |s| s.rename_workspace_by_id(ws_id, name));
            }
        }
    });

    entry.connect_activate({
        let finish = finish.clone();
        move |e| finish(Some(e.text().to_string()))
    });
    on_focus_leave(&entry, {
        let finish = finish.clone();
        move |e| finish(Some(e.text().to_string()))
    });
    let keys = gtk4::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape {
            finish(None);
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    entry.add_controller(keys);
    focus_when_shown(&entry);
}

/// Calls `f` when keyboard focus leaves `entry`. GtkEntry's own has-focus stays false (its inner
/// GtkText holds the focus), so a focus controller, which tracks the whole subtree, is used.
pub fn on_focus_leave(entry: &gtk4::Entry, f: impl Fn(&gtk4::Entry) + 'static) {
    let focus = gtk4::EventControllerFocus::new();
    let weak = entry.downgrade();
    focus.connect_leave(move |_| {
        if let Some(entry) = weak.upgrade() {
            f(&entry);
        }
    });
    entry.add_controller(focus);
}

/// Moves keyboard focus into a freshly built entry and selects its text. Deferred to an idle:
/// grab_focus does nothing until the entry is in a window, and a closing popover or the click
/// that started the rename hands focus back to the terminal first.
pub fn focus_when_shown(entry: &gtk4::Entry) {
    let weak = entry.downgrade();
    gtk4::glib::idle_add_local_once(move || {
        if let Some(entry) = weak.upgrade() {
            entry.grab_focus();
            entry.select_region(0, -1);
        }
    });
}

/// The unread-bell dot: an empty box sized by the `attention-dot` CSS (8x8, round). A label would
/// be a text line tall, and GTK before 4.12 ignores the max-height that would clamp it, so it
/// rendered as a tall oval. Centered so the row's height does not stretch it either.
pub fn attention_dot() -> gtk4::Box {
    let dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    dot.add_css_class("attention-dot");
    dot.set_valign(gtk4::Align::Center);
    dot.set_halign(gtk4::Align::Center);
    dot
}

/// Sidebar row content: GtkBox(H, 4) > [GtkBox(V, 0) > [GtkLabel(name)], GtkBox(dot), Button(close)].
/// Dot is hidden by default. The close button is hidden until the row is hovered (D-02), and is
/// wired by `wire_row` when the row is added to the sidebar.
pub fn row_content(name: &str) -> gtk4::Box {
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let label = gtk4::Label::new(Some(name));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    vbox.append(&label);
    vbox.set_hexpand(true);
    hbox.append(&vbox);

    let dot = attention_dot();
    dot.set_visible(false);
    hbox.append(&dot);

    // Close button (D-02) -- hidden by default, shown on row hover via CSS
    let close_btn = gtk4::Button::with_label("\u{00D7}"); // Unicode multiplication sign
    close_btn.add_css_class("sidebar-close-btn");
    close_btn.set_tooltip_text(Some("Close Workspace"));
    hbox.append(&close_btn);

    hbox
}

/// Wires a sidebar row for workspace `ws_id`: its close button and right-click menu. Every row
/// goes through here once, when it is added to the sidebar, so no creation path can leave a row
/// without them.
pub fn wire_row(row: &gtk4::ListBoxRow, ws_id: u64) {
    set_row_workspace_id(row, ws_id);

    let close_btn = row
        .child()
        .and_downcast::<gtk4::Box>()
        .and_then(|hbox| hbox.last_child())
        .and_then(|w| w.downcast::<gtk4::Button>().ok());
    if let Some(btn) = close_btn {
        btn.connect_clicked(move |_| {
            // Closes this row's workspace, after the usual confirmation.
            with_state(move |state| {
                let app = state.borrow().gtk_app.clone();
                crate::shortcuts::confirm_close_workspace(state, &app, ws_id);
            });
        });
    }

    let menu_model = crate::menus::build_sidebar_context_menu();
    let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
    popover.set_parent(row);
    popover.set_has_arrow(false);

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3); // Right-click
    gesture.connect_released({
        let popover = popover.clone();
        move |_, _, x, y| {
            // Select this workspace first so the menu actions apply to it.
            defer(move |s| s.select_workspace_by_id(ws_id));
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                x as i32, y as i32, 1, 1,
            )));
            popover.popup();
        }
    });
    row.add_controller(gesture);
}
