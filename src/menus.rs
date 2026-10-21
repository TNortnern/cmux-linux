use crate::app_state::{defer, with_state};
use gtk4::prelude::*;
use gtk4::gio;

/// Register all GIO actions on the ApplicationWindow.
/// Actions are named "win.{action-name}" and can be invoked by buttons, menus, and shortcuts.
/// Per D-11, D-12: all menu items mirror existing keyboard shortcut actions.
///
/// Activations run their work from a later GLib idle (`with_state` or `defer`), because an
/// activation is a GTK signal that can fire while AppState is already borrowed.
pub fn register_actions(
    window: &gtk4::ApplicationWindow,
    sidebar: &gtk4::Box,
    app: &gtk4::Application,
) {
    // --- File section actions ---

    // win.new-workspace (D-01, D-05)
    let action = gio::SimpleAction::new("new-workspace", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_new_workspace);
    });
    window.add_action(&action);

    // win.new-tab (Ctrl+Shift+T)
    let action = gio::SimpleAction::new("new-tab", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_new_tab);
    });
    window.add_action(&action);

    // win.new-ssh-workspace
    let action = gio::SimpleAction::new("new-ssh-workspace", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_new_ssh_workspace);
    });
    window.add_action(&action);

    // win.browser-open (D-07)
    let action = gio::SimpleAction::new("browser-open", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_browser_open);
    });
    window.add_action(&action);

    // win.close-pane
    let action = gio::SimpleAction::new("close-pane", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_close_pane);
    });
    window.add_action(&action);

    // win.close-workspace
    let action = gio::SimpleAction::new("close-workspace", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_close_workspace);
    });
    window.add_action(&action);

    // --- Edit section actions ---

    // win.copy (Ctrl+Shift+C) -- invoke Ghostty's copy_to_clipboard binding action
    let action = gio::SimpleAction::new("copy", None);
    action.connect_activate(|_, _| {
        defer(|s| binding_action_on_focused_pane(s, b"copy_to_clipboard"));
    });
    window.add_action(&action);

    // win.paste (Ctrl+Shift+V) -- invoke Ghostty's paste_from_clipboard binding action
    let action = gio::SimpleAction::new("paste", None);
    action.connect_activate(|_, _| {
        defer(|s| binding_action_on_focused_pane(s, b"paste_from_clipboard"));
    });
    window.add_action(&action);

    // win.find -- stub for now (terminal find not yet implemented)
    let action = gio::SimpleAction::new("find", None);
    action.set_enabled(false);
    window.add_action(&action);

    // win.preferences (D-13) -- open config.toml in $EDITOR
    let action = gio::SimpleAction::new("preferences", None);
    action.connect_activate(move |_, _| {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "xdg-open".to_string());
        let config_path = crate::config::config_path();
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !config_path.exists() {
            let _ = std::fs::write(&config_path, "# cmux configuration\n# See documentation for options\n");
        }
        if let Err(e) = std::process::Command::new(&editor).arg(&config_path).spawn() {
            cmux_warn!("could not start editor {editor}: {e}");
        }
    });
    window.add_action(&action);

    // --- View section actions ---

    // win.toggle-sidebar
    let action = gio::SimpleAction::new("toggle-sidebar", None);
    action.connect_activate({
        let sidebar = sidebar.clone();
        move |_, _| {
            let sidebar = sidebar.clone();
            defer(move |s| {
                let visible = sidebar.is_visible();
                sidebar.set_visible(!visible);
                if let Some(engine) = s.active_split_engine() {
                    engine.focus_active_surface();
                }
            });
        }
    });
    window.add_action(&action);

    // win.split-right
    let action = gio::SimpleAction::new("split-right", None);
    action.connect_activate(|_, _| {
        with_state(|state| crate::shortcuts::handle_split(state, false));
    });
    window.add_action(&action);

    // win.split-down
    let action = gio::SimpleAction::new("split-down", None);
    action.connect_activate(|_, _| {
        with_state(|state| crate::shortcuts::handle_split(state, true));
    });
    window.add_action(&action);

    // win.split-auto (Alt+Shift+D)
    let action = gio::SimpleAction::new("split-auto", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_split_auto);
    });
    window.add_action(&action);

    // win.zoom-pane (Ctrl+Shift+Z)
    let action = gio::SimpleAction::new("zoom-pane", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_zoom);
    });
    window.add_action(&action);

    // win.equalize (Ctrl+Shift+E)
    let action = gio::SimpleAction::new("equalize", None);
    action.connect_activate(|_, _| {
        with_state(crate::shortcuts::handle_equalize);
    });
    window.add_action(&action);

    // win.rename-tab (Ctrl+Shift+R)
    let action = gio::SimpleAction::new("rename-tab", None);
    action.connect_activate(|_, _| {
        defer(|s| s.begin_active_tab_rename());
    });
    window.add_action(&action);

    // win.rename-workspace (sidebar context menu)
    let action = gio::SimpleAction::new("rename-workspace", None);
    action.connect_activate(|_, _| {
        with_state(|state| {
            let list = state.borrow().sidebar_list.clone();
            crate::sidebar::start_inline_rename(&list, state);
        });
    });
    window.add_action(&action);

    // --- Help section actions ---

    // win.keyboard-shortcuts (D-14)
    let action = gio::SimpleAction::new("keyboard-shortcuts", None);
    action.connect_activate({
        let window_weak = window.downgrade();
        move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let sw = build_shortcuts_window();
                sw.set_transient_for(Some(&win));
                sw.present();
            }
        }
    });
    window.add_action(&action);

    // win.about (D-15)
    let action = gio::SimpleAction::new("about", None);
    action.connect_activate({
        let window_weak = window.downgrade();
        move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let about = gtk4::AboutDialog::builder()
                    .program_name("cmux")
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments("GPU-accelerated terminal multiplexer for Linux")
                    .website("https://github.com/manaflow-ai/cmux")
                    .license_type(gtk4::License::Agpl30)
                    .transient_for(&win)
                    .modal(true)
                    .build();
                about.present();
            }
        }
    });
    window.add_action(&action);

    // app.quit
    let quit_action = gio::SimpleAction::new("quit", None);
    quit_action.connect_activate({
        let app = app.clone();
        move |_, _| {
            app.quit();
        }
    });
    app.add_action(&quit_action);

    // --- Browser-specific actions (D-09) ---

    // win.open-external-browser -- opens current browser pane URL in xdg-open
    // Disabled until BrowserManager exposes current_url() (wired in Plan 03)
    let action = gio::SimpleAction::new("open-external-browser", None);
    action.set_enabled(false); // TODO: enable when BrowserManager.current_url() is available
    window.add_action(&action);

    // win.copy-url -- copies current browser pane URL to clipboard
    // Disabled until BrowserManager exposes current_url() (wired in Plan 03)
    let action = gio::SimpleAction::new("copy-url", None);
    action.set_enabled(false); // TODO: enable when BrowserManager.current_url() is available
    window.add_action(&action);
}

/// Sends a Ghostty binding action (for example `copy_to_clipboard`) to the focused pane.
fn binding_action_on_focused_pane(s: &crate::app_state::AppState, action: &[u8]) {
    let Some(engine) = s.active_split_engine() else {
        return;
    };
    let Some(pane_id) = engine.root.find_active_pane_id() else {
        return;
    };
    if let Some(surface) = engine.root.find_surface_for_pane(pane_id) {
        unsafe {
            crate::ghostty::ffi::ghostty_surface_binding_action(
                surface,
                action.as_ptr() as *const _,
                action.len(),
            );
        }
    }
}

/// Register keyboard accelerators for GIO actions so menus show shortcut hints.
/// Per Pitfall 3 from RESEARCH.md: GTK4 shows accels in menus ONLY if registered via set_accels_for_action.
/// Accelerators match the keymap in shortcuts.rs and the README. The capture-phase handler does the
/// work; these only show the hints in menus. Plain Ctrl+letter chords are left to the shell.
pub fn register_accels(app: &gtk4::Application) {
    app.set_accels_for_action("win.new-tab", &["<Ctrl><Shift>t"]);
    app.set_accels_for_action("win.close-pane", &["<Ctrl><Shift>w"]);
    app.set_accels_for_action("win.new-workspace", &["<Ctrl><Shift>n"]);
    app.set_accels_for_action("win.new-ssh-workspace", &["<Ctrl><Shift>s"]);
    app.set_accels_for_action("win.browser-open", &["<Ctrl><Alt>b"]);
    app.set_accels_for_action("win.toggle-sidebar", &["<Ctrl><Shift>b"]);
    app.set_accels_for_action("win.split-right", &["<Alt><Shift>plus"]);
    app.set_accels_for_action("win.split-down", &["<Alt><Shift>underscore"]);
    app.set_accels_for_action("win.split-auto", &["<Alt><Shift>d"]);
    app.set_accels_for_action("win.zoom-pane", &["<Ctrl><Shift>z"]);
    app.set_accels_for_action("win.equalize", &["<Ctrl><Shift>e"]);
    app.set_accels_for_action("win.copy", &["<Ctrl><Shift>c"]);
    app.set_accels_for_action("win.paste", &["<Ctrl><Shift>v"]);
    app.set_accels_for_action("win.find", &["<Ctrl><Shift>f"]);
    app.set_accels_for_action("win.rename-tab", &["<Ctrl><Shift>r"]);
    app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
    app.set_accels_for_action("app.quit", &["<Ctrl><Shift>q"]);
}

/// Build the hamburger menu model (D-11, D-12).
/// Returns a gio::Menu that can be set on a MenuButton.
/// Per D-12: sections use File/Edit/View/Help labels.
pub fn build_hamburger_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // File section (D-12)
    let file_section = gio::Menu::new();
    file_section.append(Some("New Tab"), Some("win.new-tab"));
    file_section.append(Some("New Workspace"), Some("win.new-workspace"));
    file_section.append(Some("New SSH Workspace"), Some("win.new-ssh-workspace"));
    file_section.append(Some("New Browser"), Some("win.browser-open"));
    file_section.append(Some("Close Pane"), Some("win.close-pane"));
    file_section.append(Some("Close Workspace"), Some("win.close-workspace"));
    file_section.append(Some("Quit"), Some("app.quit"));
    menu.append_section(Some("File"), &file_section);

    // Edit section (D-12)
    let edit_section = gio::Menu::new();
    edit_section.append(Some("Copy"), Some("win.copy"));
    edit_section.append(Some("Paste"), Some("win.paste"));
    edit_section.append(Some("Find"), Some("win.find"));
    edit_section.append(Some("Preferences"), Some("win.preferences"));
    menu.append_section(Some("Edit"), &edit_section);

    // View section (D-12)
    let view_section = gio::Menu::new();
    view_section.append(Some("Toggle Sidebar"), Some("win.toggle-sidebar"));
    view_section.append(Some("Rename Tab"), Some("win.rename-tab"));
    view_section.append(Some("Split Right"), Some("win.split-right"));
    view_section.append(Some("Split Down"), Some("win.split-down"));
    view_section.append(Some("Split Along Longer Side"), Some("win.split-auto"));
    view_section.append(Some("Zoom Pane"), Some("win.zoom-pane"));
    view_section.append(Some("Equalize Panes"), Some("win.equalize"));
    menu.append_section(Some("View"), &view_section);

    // Help section (D-12)
    let help_section = gio::Menu::new();
    help_section.append(Some("Keyboard Shortcuts"), Some("win.keyboard-shortcuts"));
    help_section.append(Some("About cmux"), Some("win.about"));
    menu.append_section(Some("Help"), &help_section);

    menu
}

/// Build sidebar workspace row context menu (D-03).
pub fn build_sidebar_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Rename"), Some("win.rename-workspace"));
    menu.append(Some("Close"), Some("win.close-workspace"));
    menu.append(Some("Split Right"), Some("win.split-right"));
    menu.append(Some("Split Down"), Some("win.split-down"));
    menu
}

/// Build terminal pane context menu (D-08).
pub fn build_terminal_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let edit_section = gio::Menu::new();
    edit_section.append(Some("Copy"), Some("win.copy"));
    edit_section.append(Some("Paste"), Some("win.paste"));
    menu.append_section(None, &edit_section);

    let pane_section = gio::Menu::new();
    pane_section.append(Some("Split Right"), Some("win.split-right"));
    pane_section.append(Some("Split Down"), Some("win.split-down"));
    pane_section.append(Some("Close Pane"), Some("win.close-pane"));
    menu.append_section(None, &pane_section);

    let browser_section = gio::Menu::new();
    browser_section.append(Some("Open Browser Here"), Some("win.browser-open"));
    menu.append_section(None, &browser_section);

    menu
}

/// Build browser preview pane context menu (D-09).
/// Includes Open in External Browser and Copy URL actions.
pub fn build_browser_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Open in External Browser"), Some("win.open-external-browser"));
    menu.append(Some("Copy URL"), Some("win.copy-url"));
    menu.append(Some("Close Pane"), Some("win.close-pane"));
    menu
}

/// Keyboard shortcuts shown in the help window. Mirrors the defaults in config.rs (ShortcutConfig).
const SHORTCUT_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Tabs", &[
        ("<Ctrl><Shift>t", "New Tab"),
        ("<Ctrl><Shift>w", "Close Pane (closes the tab on its last pane)"),
        ("<Ctrl>Tab", "Next Tab"),
        ("<Ctrl><Shift>Tab", "Previous Tab"),
        ("<Ctrl>1..8", "Select Tab 1-8"),
        ("<Ctrl>9", "Last Tab"),
        ("<Ctrl><Shift>r", "Rename Tab"),
    ]),
    ("Panes", &[
        ("<Alt><Shift>plus", "Split Right"),
        ("<Alt><Shift>underscore", "Split Down"),
        ("<Alt><Shift>d", "Split Along Longer Side"),
        ("<Alt>Left", "Focus Left"),
        ("<Alt>Right", "Focus Right"),
        ("<Alt>Up", "Focus Up"),
        ("<Alt>Down", "Focus Down"),
        ("<Alt><Shift>Left", "Resize Left"),
        ("<Alt><Shift>Right", "Resize Right"),
        ("<Alt><Shift>Up", "Resize Up"),
        ("<Alt><Shift>Down", "Resize Down"),
        ("<Ctrl><Shift>z", "Zoom Pane"),
        ("<Ctrl><Shift>e", "Equalize Panes"),
    ]),
    ("Workspaces", &[
        ("<Ctrl><Shift>n", "New Workspace"),
        ("<Ctrl><Alt>1..9", "Select Workspace 1-9"),
        ("<Ctrl><Shift>s", "New SSH Workspace"),
    ]),
    ("View", &[
        ("<Ctrl><Shift>b", "Toggle Sidebar"),
        ("<Ctrl><Alt>b", "Open Browser"),
        ("<Ctrl><Alt><Shift>b", "Close Browser"),
    ]),
    ("Edit", &[
        ("<Ctrl><Shift>c", "Copy"),
        ("<Ctrl><Shift>v", "Paste"),
        ("<Ctrl><Shift>f", "Find"),
    ]),
    ("General", &[
        ("<Ctrl>comma", "Preferences"),
        ("<Ctrl><Shift>q", "Quit"),
    ]),
];

/// Build GtkShortcutsWindow (D-14) with all shortcuts grouped by category.
/// Built from GtkBuilder XML: the add_section/add_group API needs GTK 4.14, while XML works on
/// every GTK 4 (Ubuntu 22.04 ships 4.6).
fn build_shortcuts_window() -> gtk4::ShortcutsWindow {
    gtk4::Builder::from_string(&shortcuts_ui_xml())
        .object("shortcuts")
        .expect("shortcuts window in generated UI")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// GtkBuilder UI for the shortcuts window, generated from SHORTCUT_GROUPS.
fn shortcuts_ui_xml() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<interface>
<object class=\"GtkShortcutsWindow\" id=\"shortcuts\">
",
    );
    for (title, shortcuts) in SHORTCUT_GROUPS {
        xml.push_str(&format!(
            "<child><object class=\"GtkShortcutsSection\"><property name=\"section-name\">{}</property><property name=\"title\">{}</property>
<child><object class=\"GtkShortcutsGroup\"><property name=\"title\">{}</property>
",
            xml_escape(&title.to_lowercase()),
            xml_escape(title),
            xml_escape(title)
        ));
        for (accel, label) in shortcuts.iter() {
            xml.push_str(&format!(
                "<child><object class=\"GtkShortcutsShortcut\"><property name=\"accelerator\">{}</property><property name=\"title\">{}</property></object></child>
",
                xml_escape(accel),
                xml_escape(label)
            ));
        }
        xml.push_str("</object></child>
</object></child>
");
    }
    xml.push_str("</object>
</interface>
");
    xml
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcuts_ui_xml_is_escaped_and_lists_every_shortcut() {
        let xml = super::shortcuts_ui_xml();
        assert!(xml.contains("&lt;Ctrl&gt;&lt;Shift&gt;c"));
        assert!(!xml.contains("<Ctrl>"));
        let count: usize = super::SHORTCUT_GROUPS.iter().map(|(_, s)| s.len()).sum();
        assert_eq!(xml.matches("GtkShortcutsShortcut").count(), count);
    }
}
