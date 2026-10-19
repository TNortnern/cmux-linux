//! Tab strip above a workspace's splits. Tab widgets are updated in place when the tab list
//! does not change, so a double click lands on the same widget. Every action is queued with
//! `app_state::defer`: a GTK signal can fire while AppState is already borrowed, and the queued
//! action then runs on the next idle instead.
use crate::app_state::defer;
use gtk4::glib::prelude::{StaticType, ToValue};
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};

/// One tab button and the label inside it.
struct Item {
    id: u64,
    widget: gtk4::Box,
    label: gtk4::Label,
}

pub struct TabBar {
    ws_id: u64,
    /// Outer box: the scrolling strip and the "+" button.
    pub root: gtk4::Box,
    strip: gtk4::Box,
    items: RefCell<Vec<Item>>,
    /// Tab whose name is being edited. Set by AppState, which also clears it.
    renaming: Cell<Option<u64>>,
    /// The `renaming` value the current items were built with.
    built_rename: Cell<Option<u64>>,
}

impl TabBar {
    pub fn new(ws_id: u64) -> Self {
        let strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        strip.add_css_class("tab-strip");

        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
        scroller.set_hexpand(true);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&strip));

        let add = gtk4::Button::with_label("+");
        add.add_css_class("tab-add");
        add.set_tooltip_text(Some("New Tab"));
        add.connect_clicked(|_| defer(|s| {
            s.new_tab();
        }));

        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.add_css_class("tab-bar");
        root.append(&scroller);
        root.append(&add);

        Self {
            ws_id,
            root,
            strip,
            items: RefCell::new(Vec::new()),
            renaming: Cell::new(None),
            built_rename: Cell::new(None),
        }
    }

    /// Tab whose name is being edited, if any.
    pub fn renaming(&self) -> Option<u64> {
        self.renaming.get()
    }

    pub fn set_renaming(&self, tab: Option<u64>) {
        self.renaming.set(tab);
    }

    /// Shows `tabs` (id, name) in order, with the tab `active` selected.
    pub fn set_tabs(&self, tabs: &[(u64, String)], active: u64) {
        let rename = self.renaming.get();
        let same_order = {
            let items = self.items.borrow();
            items.len() == tabs.len()
                && items.iter().zip(tabs).all(|(item, (id, _))| item.id == *id)
        };
        if same_order && rename == self.built_rename.get() {
            let items = self.items.borrow();
            for (item, (_, name)) in items.iter().zip(tabs) {
                item.label.set_text(name);
                if item.id == active {
                    item.widget.add_css_class("tab-active");
                } else {
                    item.widget.remove_css_class("tab-active");
                }
            }
        } else {
            self.rebuild(tabs, active, rename);
        }
    }

    fn rebuild(&self, tabs: &[(u64, String)], active: u64, rename: Option<u64>) {
        while let Some(child) = self.strip.first_child() {
            self.strip.remove(&child);
        }
        let mut items = self.items.borrow_mut();
        items.clear();
        for (id, name) in tabs {
            let item = self.build_item(*id, name, *id == active, rename == Some(*id));
            self.strip.append(&item.widget);
            items.push(item);
        }
        self.built_rename.set(rename);
    }

    fn build_item(&self, id: u64, name: &str, active: bool, editing: bool) -> Item {
        let ws_id = self.ws_id;

        let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        widget.add_css_class("tab");
        if active {
            widget.add_css_class("tab-active");
        }

        let label = gtk4::Label::new(Some(name));
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(24);

        if editing {
            let entry = gtk4::Entry::new();
            entry.set_text(name);
            entry.set_width_chars(16);
            entry.add_css_class("tab-rename");
            entry.connect_activate(move |e| {
                let text = e.text().to_string();
                defer(move |s| s.end_tab_rename(ws_id, id, Some(text)));
            });
            // Focus-out commits the edit. A rename that already ended is ignored by AppState.
            entry.connect_has_focus_notify(move |e| {
                if !e.has_focus() {
                    let text = e.text().to_string();
                    defer(move |s| s.end_tab_rename(ws_id, id, Some(text)));
                }
            });
            let keys = gtk4::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    defer(move |s| s.end_tab_rename(ws_id, id, None));
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            entry.add_controller(keys);
            widget.append(&entry);
            entry.grab_focus();
        } else {
            widget.append(&label);
        }

        let close = gtk4::Button::with_label("\u{00D7}");
        close.add_css_class("tab-close");
        close.set_tooltip_text(Some("Close Tab"));
        close.connect_clicked(move |_| {
            defer(move |s| {
                s.close_tab_by_id(ws_id, id);
            })
        });
        widget.append(&close);

        // Left click selects; double click renames; middle click closes.
        let left = gtk4::GestureClick::new();
        left.set_button(1);
        left.connect_pressed(move |_, n_press, _, _| match n_press {
            1 => defer(move |s| s.select_tab_by_id(ws_id, id)),
            2 => defer(move |s| s.begin_tab_rename(ws_id, id)),
            _ => {}
        });
        widget.add_controller(left);

        let middle = gtk4::GestureClick::new();
        middle.set_button(2);
        middle.connect_pressed(move |_, _, _, _| {
            defer(move |s| {
                s.close_tab_by_id(ws_id, id);
            })
        });
        widget.add_controller(middle);

        // Drag a tab onto another to reorder. The payload is the dragged tab's id.
        let drag = gtk4::DragSource::new();
        drag.set_actions(gtk4::gdk::DragAction::MOVE);
        drag.connect_prepare(move |_, _, _| Some(gtk4::gdk::ContentProvider::for_value(&id.to_value())));
        widget.add_controller(drag);

        let drop = gtk4::DropTarget::new(u64::static_type(), gtk4::gdk::DragAction::MOVE);
        drop.connect_drop(move |_, value, _, _| match value.get::<u64>() {
            Ok(from) => {
                defer(move |s| s.move_tab_by_id(ws_id, from, id));
                true
            }
            Err(_) => false,
        });
        widget.add_controller(drop);

        Item { id, widget, label }
    }
}
