#[allow(deprecated)]
use gtk4::prelude::*;
use crate::app_state::{defer, AppState};

/// Show the SSH connect dialog with autocomplete from saved hosts and SSH config.
/// Connecting and the focus restore run through `defer`, so they never run inside a borrow of
/// AppState.
pub fn show_ssh_dialog(app: &gtk4::Application) {
    let window = app.windows().into_iter().next();

    let dialog = gtk4::Window::builder()
        .title("Connect to SSH Host")
        .modal(true)
        .default_width(400)
        .build();
    dialog.add_css_class("ssh-dialog");

    if let Some(ref parent) = window {
        dialog.set_transient_for(Some(parent));
    }

    // Entry with placeholder
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("user@host or SSH alias"));

    // EntryCompletion with merged host list
    #[allow(deprecated)]
    {
        let store = gtk4::ListStore::new(&[gtk4::glib::Type::STRING]);
        for host in crate::ssh_hosts::all_hosts() {
            store.set(&store.append(), &[(0, &host)]);
        }
        let completion = gtk4::EntryCompletion::new();
        completion.set_model(Some(&store));
        completion.set_text_column(0);
        completion.set_minimum_key_length(1);
        entry.set_completion(Some(&completion));
    }

    // Layout: vertical box with padding
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.append(&entry);
    dialog.set_child(Some(&vbox));

    // Enter key: create SSH workspace
    entry.connect_activate({
        let dialog = dialog.clone();
        move |e| {
            let target = e.text().to_string().trim().to_string();
            if target.is_empty() {
                return;
            }
            // Reject unsafe targets here; the dialog stays open so the user can fix the text.
            if let Err(msg) = crate::ssh::target::validate_target(&target) {
                cmux_warn!("invalid SSH target: {msg}");
                return;
            }
            defer(move |s| {
                connect_remote(s, target);
            });
            dialog.close();
        }
    });

    // Escape key: close dialog
    let esc_ctrl = gtk4::EventControllerKey::new();
    esc_ctrl.connect_key_pressed({
        let dialog = dialog.clone();
        move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                gtk4::glib::Propagation::Stop
            } else {
                gtk4::glib::Propagation::Proceed
            }
        }
    });
    dialog.add_controller(esc_ctrl);

    // Restore terminal focus after dialog closes
    dialog.connect_destroy(move |_| {
        defer(|s| {
            if let Some(engine) = s.active_split_engine() {
                engine.focus_active_surface();
            }
        });
    });

    dialog.present();
    entry.grab_focus();
}

/// Creates the SSH workspace for `target`, wires its bridge and starts its lifecycle task.
/// Same steps as the socket's workspace.create with a remote target.
fn connect_remote(s: &mut AppState, target: String) {
    let (write_tx, write_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, _output_rx) = tokio::sync::mpsc::unbounded_channel();
    let bridge = std::sync::Arc::new(crate::ssh::bridge::SshBridge::new(write_tx, write_rx, output_tx));
    let id = s.create_remote_workspace(target.clone(), &bridge);
    s.workspace_bridges.insert(id, bridge.clone());

    if let (Some(tx), Some(rt)) = (s.ssh_event_tx.clone(), s.runtime_handle.clone()) {
        let handle = rt.spawn(crate::ssh::tunnel::run_ssh_lifecycle(id, target, tx, bridge));
        s.ssh_task_handles.insert(id, handle);
    }
}
