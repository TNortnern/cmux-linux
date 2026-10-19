use crate::ghostty::ffi;
use crate::ghostty::surface::{teardown_pane, PaneCtx, PaneSurface, SurfaceCell};
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

/// Process-wide pane ID counter. IDs start at 1 because 0 means "no pane" (BELL_PANE_ID).
static NEXT_PANE_ID: AtomicU64 = AtomicU64::new(1);

/// Allocates a pane ID that is unique across all workspaces and restored sessions.
pub fn alloc_pane_id() -> u64 {
    NEXT_PANE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Outcome of closing a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseResult {
    /// The pane is gone. Carries the pane that is active in this workspace afterwards.
    Closed(u64),
    /// The pane is the workspace's only terminal (or its only pane). Nothing was changed;
    /// the caller decides whether to close the whole workspace.
    LastPane,
    /// The pane does not exist, or the tree is in an unexpected state. Nothing was changed.
    Failed,
}

/// Direction for pane focus navigation (Ctrl+Shift+arrows per D-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Recursive pane layout tree. Each workspace has one root SplitNode.
/// - Leaf: a single terminal pane (GtkGLArea + Ghostty surface)
/// - Split: two child subtrees separated by a GtkPaned divider
///
/// Per SPLIT-06: this is the Bonsplit Rust port — immutable-style tree where
/// split/close operations return a new root.
#[derive(Clone)]
pub enum SplitNode {
    Leaf {
        pane_id: u64,
        gl_area: gtk4::GLArea,
        /// The pane's live Ghostty surface. Filled by the realize callback and cleared by
        /// teardown. Read the surface only through this handle.
        surface: SurfaceCell,
        /// Per-pane context shared with Ghostty as surface userdata. Freed by teardown_pane.
        ctx: *mut PaneCtx,
        /// Stable UUID for session persistence and v2 socket protocol pane identity.
        uuid: Uuid,
        /// Phase 4 NOTF-01: true when this pane has unread bell activity.
        has_attention: bool,
    },
    /// Phase 8: Browser preview pane (agent-browser frame rendering).
    Preview {
        pane_id: u64,
        container: gtk4::Box,
        picture: gtk4::Picture,
        #[allow(dead_code)] // Kept alive for GTK widget tree reference counting
        url_entry: gtk4::Entry,
        uuid: Uuid,
    },
    Split {
        orientation: gtk4::Orientation,
        paned: gtk4::Paned,
        start: Box<SplitNode>,
        end: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Returns the root GTK widget for this node (GLArea for Leaf, Overlay for Preview, Paned for Split).
    pub fn widget(&self) -> gtk4::Widget {
        match self {
            SplitNode::Leaf { gl_area, .. } => gl_area.clone().upcast(),
            SplitNode::Preview { container, .. } => container.clone().upcast(),
            SplitNode::Split { paned, .. } => paned.clone().upcast(),
        }
    }

    /// Find the pane_id of the active (focused) leaf by checking CSS class.
    pub fn find_active_pane_id(&self) -> Option<u64> {
        match self {
            SplitNode::Leaf {
                pane_id, gl_area, ..
            } => {
                if gl_area.has_css_class("active-pane") {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Preview { pane_id, container, .. } => {
                if container.has_css_class("active-pane") {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => start
                .find_active_pane_id()
                .or_else(|| end.find_active_pane_id()),
        }
    }

    /// Find the UUID for a pane by pane_id. Returns None if not found.
    pub fn find_uuid_for_pane(&self, target_id: u64) -> Option<String> {
        match self {
            SplitNode::Leaf { pane_id, uuid, .. } | SplitNode::Preview { pane_id, uuid, .. } => {
                if *pane_id == target_id {
                    Some(uuid.to_string())
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => {
                start.find_uuid_for_pane(target_id)
                    .or_else(|| end.find_uuid_for_pane(target_id))
            }
        }
    }

    /// Apply the active-pane CSS class to the leaf matching active_pane_id.
    /// Removes the class from all other leaves.
    pub fn update_focus_css(&self, active_pane_id: u64) {
        match self {
            SplitNode::Leaf {
                pane_id, gl_area, ..
            } => {
                if *pane_id == active_pane_id {
                    gl_area.add_css_class("active-pane");
                } else {
                    gl_area.remove_css_class("active-pane");
                }
            }
            SplitNode::Preview { pane_id, container, .. } => {
                if *pane_id == active_pane_id {
                    container.add_css_class("active-pane");
                } else {
                    container.remove_css_class("active-pane");
                }
            }
            SplitNode::Split { start, end, .. } => {
                start.update_focus_css(active_pane_id);
                end.update_focus_css(active_pane_id);
            }
        }
    }

    /// Find a node by pane_id.
    pub fn find_node(&self, target_id: u64) -> Option<&SplitNode> {
        match self {
            SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => {
                if *pane_id == target_id { Some(self) } else { None }
            }
            SplitNode::Split { start, end, .. } => {
                start.find_node(target_id).or_else(|| end.find_node(target_id))
            }
        }
    }

    /// Find the Ghostty surface handle for a specific pane by pane_id.
    /// Used by debug.type to send text to a specific pane's surface.
    pub fn find_surface_for_pane(&self, target_id: u64) -> Option<ffi::ghostty_surface_t> {
        match self {
            SplitNode::Leaf { pane_id, surface, .. } => {
                if *pane_id == target_id { *surface.borrow() } else { None }
            }
            SplitNode::Preview { .. } => None, // No Ghostty surface
            SplitNode::Split { start, end, .. } => {
                start.find_surface_for_pane(target_id)
                    .or_else(|| end.find_surface_for_pane(target_id))
            }
        }
    }

    /// Collect (uuid, pane_id, active) for all leaves in this subtree.
    pub fn collect_pane_info(&self, out: &mut Vec<(Uuid, u64, bool)>, active_id: u64) {
        match self {
            SplitNode::Leaf { pane_id, uuid, .. } | SplitNode::Preview { pane_id, uuid, .. } => {
                out.push((*uuid, *pane_id, *pane_id == active_id));
            }
            SplitNode::Split { start, end, .. } => {
                start.collect_pane_info(out, active_id);
                end.collect_pane_info(out, active_id);
            }
        }
    }

    /// Find the ghostty surface handle for the leaf matching target_uuid (UUID string).
    pub fn find_by_uuid(&self, target_uuid: &str) -> Option<ffi::ghostty_surface_t> {
        match self {
            SplitNode::Leaf { uuid, surface, .. } => {
                if uuid.to_string() == target_uuid { *surface.borrow() } else { None }
            }
            SplitNode::Preview { .. } => None, // No Ghostty surface to return
            SplitNode::Split { start, end, .. } => {
                start.find_by_uuid(target_uuid).or_else(|| end.find_by_uuid(target_uuid))
            }
        }
    }

    /// Find the pane_id for the leaf matching target_uuid (UUID string).
    pub fn find_pane_id_by_uuid(&self, target_uuid: &str) -> Option<u64> {
        match self {
            SplitNode::Leaf { uuid, pane_id, .. } | SplitNode::Preview { uuid, pane_id, .. } => {
                if uuid.to_string() == target_uuid { Some(*pane_id) } else { None }
            }
            SplitNode::Split { start, end, .. } => {
                start.find_pane_id_by_uuid(target_uuid)
                    .or_else(|| end.find_pane_id_by_uuid(target_uuid))
            }
        }
    }

    /// Set has_attention on the leaf matching pane_id. Returns true if found.
    pub fn set_attention(&mut self, target_pane_id: u64, value: bool) -> bool {
        match self {
            SplitNode::Leaf { pane_id, has_attention, .. } => {
                if *pane_id == target_pane_id {
                    *has_attention = value;
                    true
                } else {
                    false
                }
            }
            SplitNode::Preview { .. } => false, // No attention state
            SplitNode::Split { start, end, .. } => {
                start.set_attention(target_pane_id, value) || end.set_attention(target_pane_id, value)
            }
        }
    }

    /// Returns true if any leaf in this subtree has attention.
    pub fn any_attention(&self) -> bool {
        match self {
            SplitNode::Leaf { has_attention, .. } => *has_attention,
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => start.any_attention() || end.any_attention(),
        }
    }

    /// Check if a specific pane has attention.
    pub fn pane_has_attention(&self, target_pane_id: u64) -> bool {
        match self {
            SplitNode::Leaf { pane_id, has_attention, .. } => {
                *pane_id == target_pane_id && *has_attention
            }
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => {
                start.pane_has_attention(target_pane_id) || end.pane_has_attention(target_pane_id)
            }
        }
    }

    /// Clear attention on all leaves in this subtree.
    pub fn clear_all_attention(&mut self) {
        match self {
            SplitNode::Leaf { has_attention, .. } => *has_attention = false,
            SplitNode::Preview { .. } => {} // No attention state
            SplitNode::Split { start, end, .. } => {
                start.clear_all_attention();
                end.clear_all_attention();
            }
        }
    }
}

/// Attach right-click context menu to a terminal GLArea (D-08).
/// Uses button 3 (right-click only) to avoid interfering with Ghostty's mouse handling.
fn attach_terminal_context_menu(gl_area: &gtk4::GLArea) {
    let menu_model = crate::menus::build_terminal_context_menu();
    let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
    popover.set_parent(gl_area);
    popover.set_has_arrow(false);

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3); // Right-click only
    gesture.connect_released({
        let popover = popover.clone();
        move |_, _, x, y| {
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                x as i32, y as i32, 1, 1,
            )));
            popover.popup();
        }
    });
    gl_area.add_controller(gesture);
}

/// SplitEngine manages one workspace's pane layout tree.
pub struct SplitEngine {
    pub root: SplitNode,
    pub active_pane_id: u64,
    /// GTK Application handle needed to create new GLAreas.
    app: gtk4::Application,
    /// Ghostty app handle needed to create new surfaces.
    ghostty_app: ffi::ghostty_app_t,
}

impl SplitEngine {
    /// Create a new SplitEngine with a single leaf pane.
    /// `pane` comes from `create_surface`; `pane_id` must be the id it was created with.
    pub fn new(
        app: gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        pane: PaneSurface,
        pane_id: u64,
    ) -> Self {
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&pane.gl_area);
        let root = SplitNode::Leaf {
            pane_id,
            gl_area: pane.gl_area,
            surface: pane.surface,
            ctx: pane.ctx,
            uuid: Uuid::new_v4(),
            has_attention: false,
        };
        // Mark the pane active so the focus-gated clipboard write path accepts it.
        root.update_focus_css(pane_id);
        SplitEngine {
            root,
            active_pane_id: pane_id,
            app,
            ghostty_app,
        }
    }

    /// Reconstruct a SplitEngine from serialized SplitNodeData (D-05).
    /// Creates GTK widgets (GLArea for leaves, Paned for splits) and Ghostty surfaces.
    /// Fresh pane_ids are generated (D-06), but UUIDs are preserved from session.
    /// Returns None if tree_depth > 16 (D-14). On None, every GLArea created so far is torn down.
    pub fn from_data(
        app: gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        active_pane_uuid: Option<&str>,
    ) -> Option<Self> {
        let root = Self::node_from_data(&app, ghostty_app, data, 0)?;
        // Find active pane by saved UUID, or fall back to first leaf
        let active_id = active_pane_uuid
            .and_then(|uuid_str| root.find_pane_id_by_uuid(uuid_str))
            .unwrap_or_else(|| first_pane_id(&root));
        root.update_focus_css(active_id);
        Some(SplitEngine {
            root,
            active_pane_id: active_id,
            app,
            ghostty_app,
        })
    }

    fn node_from_data(
        app: &gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        depth: u32,
    ) -> Option<SplitNode> {
        if depth > 16 {
            eprintln!("cmux: session restore tree depth > 16, falling back (D-14)");
            return None;
        }
        match data {
            SplitNodeData::Leaf { surface_uuid, .. } => {
                let pane_id = alloc_pane_id();
                // The realize callback creates the Ghostty surface and fills the leaf's handle.
                let pane = crate::ghostty::surface::create_surface(
                    app,
                    ghostty_app,
                    None,
                    pane_id,
                    crate::ghostty::surface::SurfaceIoMode::Exec,
                );
                // Phase 9: Attach right-click context menu (D-08)
                attach_terminal_context_menu(&pane.gl_area);
                // D-06: preserve UUID from session
                Some(SplitNode::Leaf {
                    pane_id,
                    gl_area: pane.gl_area,
                    surface: pane.surface,
                    ctx: pane.ctx,
                    uuid: *surface_uuid,
                    has_attention: false,
                })
            }
            SplitNodeData::Split { orientation, ratio, start, end } => {
                let start_node = Self::node_from_data(app, ghostty_app, start, depth + 1)?;
                let end_node = match Self::node_from_data(app, ghostty_app, end, depth + 1) {
                    Some(n) => n,
                    None => {
                        teardown_leaves(&start_node);
                        return None;
                    }
                };
                let gtk_orientation = match orientation.as_str() {
                    "vertical" => gtk4::Orientation::Vertical,
                    _ => gtk4::Orientation::Horizontal,
                };
                let paned = gtk4::Paned::new(gtk_orientation);
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                paned.set_wide_handle(true);
                paned.set_start_child(Some(&start_node.widget()));
                paned.set_end_child(Some(&end_node.widget()));
                // D-03: restore ratio after layout pass
                let saved_ratio = *ratio;
                let paned_ref = paned.clone();
                let orient = gtk_orientation;
                gtk4::glib::idle_add_local_once(move || {
                    let size = if orient == gtk4::Orientation::Horizontal {
                        paned_ref.width()
                    } else {
                        paned_ref.height()
                    };
                    if size > 0 {
                        paned_ref.set_position((size as f64 * saved_ratio) as i32);
                    }
                });
                Some(SplitNode::Split {
                    orientation: gtk_orientation,
                    paned,
                    start: Box::new(start_node),
                    end: Box::new(end_node),
                })
            }
        }
    }

    /// No-op. Surfaces are recorded in each leaf's handle by the GLArea realize callback, so
    /// there is nothing to sync from the registry. Kept only because main.rs still calls it.
    pub fn sync_surfaces_from_registry(&mut self) {}

    /// Returns the root widget of this workspace's split tree.
    pub fn root_widget(&self) -> gtk4::Widget {
        self.root.widget()
    }

    /// Grab GTK keyboard focus for the active pane's GLArea.
    /// Called after workspace switch so key events route to Ghostty, not the sidebar.
    pub fn grab_active_focus(&self) {
        if let Some(gl_area) = self.find_gl_area(self.active_pane_id) {
            gl_area.grab_focus();
        }
    }

    /// Returns the UUID of the currently active pane, if found.
    pub fn active_pane_uuid(&self) -> Option<String> {
        self.root.find_uuid_for_pane(self.active_pane_id)
    }

    /// Grab GTK keyboard focus AND notify Ghostty of focus for the active pane.
    /// Use this after any operation that may have moved focus away from the terminal
    /// (sidebar toggle, workspace switch, etc.). grab_active_focus() only handles the
    /// GTK side; this method ensures Ghostty's internal focused state is also updated.
    pub fn focus_active_surface(&self) {
        if let Some(gl_area) = self.find_gl_area(self.active_pane_id) {
            gl_area.grab_focus();
        }
        if let Some(surface) = self.find_surface(self.active_pane_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(surface, true);
            }
        }
        // Kick the render loop to repaint after focus restore.
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if area.is_realized() {
                    area.queue_render();
                }
            }
        }
    }

    /// Split the active pane to the right (Ctrl+D per D-10).
    /// Replaces the active Leaf with a Split(Horizontal) containing the old leaf + new leaf.
    /// Per D-08: new surface inherits CWD via ghostty_surface_inherited_config.
    /// Per D-09: initial split ratio is 50/50 (set in paned.connect_realize).
    /// Per SPLIT-07: new pane receives focus immediately.
    pub fn split_right(&mut self) -> Option<u64> {
        self.split_active(gtk4::Orientation::Horizontal)
    }

    /// Split the active pane downward (Ctrl+Shift+D per D-10).
    pub fn split_down(&mut self) -> Option<u64> {
        self.split_active(gtk4::Orientation::Vertical)
    }

    pub fn split_active(&mut self, orientation: gtk4::Orientation) -> Option<u64> {
        let active_id = self.active_pane_id;

        // When the root is a Leaf (first split), the GLArea is a direct child of the GtkStack
        // page. The replacer will remove it from the Stack (via remove_widget_from_parent) and
        // place it inside the new Paned. We then need to add the Paned to the Stack page.
        // Only capture this for Leaf roots — for nested splits the outer Paned stays in the Stack.
        let old_root_widget = self.root.widget();
        let stack_slot: Option<(gtk4::Stack, String)> =
            if matches!(self.root, SplitNode::Leaf { .. } | SplitNode::Preview { .. }) {
                old_root_widget
                    .parent()
                    .and_then(|p| p.downcast::<gtk4::Stack>().ok())
                    .and_then(|stack| {
                        let name = stack.page(&old_root_widget).name()?.to_string();
                        Some((stack, name))
                    })
            } else {
                None
            };

        // Find the active leaf's surface for inherited config. None if it is not realized yet.
        let inherited_surface = self.find_surface(active_id)?;

        // Unfocus the old surface before the split — Ghostty routes input by focus state,
        // so without this the old pane continues receiving keystrokes after the new pane
        // is created (SPLIT-07).
        unsafe {
            ffi::ghostty_surface_set_focus(inherited_surface, false);
        }

        // Get inherited config from the active surface (for CWD inheritance per D-08).
        // Pass by value (ghostty_surface_config_s is Copy) — avoids dangling pointer
        // in the GLArea realize callback, which fires asynchronously after this returns.
        let inherited_config = unsafe {
            ffi::ghostty_surface_inherited_config(
                inherited_surface,
                ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_SPLIT,
            )
        };

        // Create new GLArea + surface for the new pane.
        let new_pane_id = alloc_pane_id();
        let pane = crate::ghostty::surface::create_surface(
            &self.app,
            self.ghostty_app,
            Some(inherited_config),
            new_pane_id,
            crate::ghostty::surface::SurfaceIoMode::Exec,
        );
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&pane.gl_area);
        let new_gl_area = pane.gl_area.clone();

        // Replace the active leaf in the tree with a Split node.
        let new_leaf = SplitNode::Leaf {
            pane_id: new_pane_id,
            gl_area: pane.gl_area,
            surface: pane.surface,
            ctx: pane.ctx,
            uuid: Uuid::new_v4(),
            has_attention: false,
        };

        let _replaced = self.replace_leaf_with_split(active_id, new_leaf, orientation)?;

        // If the root was a Leaf, it's now a Split whose Paned has no parent.
        // Re-parent the new Paned root into the GtkStack page we saved above.
        if let Some((stack, name)) = stack_slot {
            let new_root = self.root.widget();
            stack.add_named(&new_root, Some(&name));
            stack.set_visible_child_name(&name);
        }

        // After realize, update active focus to the new pane.
        self.active_pane_id = new_pane_id;
        self.root.update_focus_css(new_pane_id);

        // Focus the new GLArea widget so it receives keyboard events.
        new_gl_area.grab_focus();

        Some(new_pane_id)
    }

    /// Split the active pane vertically and insert a Preview node on the right.
    /// Returns (new_pane_id, picture_widget, url_entry) so the caller can wire streaming and URL navigation.
    /// The active terminal pane stays on the left and retains focus.
    pub fn split_active_with_preview(&mut self) -> Option<crate::browser::PreviewPaneWidgets> {
        let active_id = self.active_pane_id;
        let new_pane_id = alloc_pane_id();

        // Same stack re-parenting guard as split_active:
        // When root is a Leaf or Preview, capture the GtkStack parent so we can
        // re-parent the new Paned into the Stack after the split.
        let old_root_widget = self.root.widget();
        let stack_slot: Option<(gtk4::Stack, String)> =
            if matches!(self.root, SplitNode::Leaf { .. } | SplitNode::Preview { .. }) {
                old_root_widget
                    .parent()
                    .and_then(|p| p.downcast::<gtk4::Stack>().ok())
                    .and_then(|stack| {
                        let name = stack.page(&old_root_widget).name()?.to_string();
                        Some((stack, name))
                    })
            } else {
                None
            };

        // Create preview pane widgets
        let widgets = crate::browser::create_preview_pane(new_pane_id);

        // Phase 9: Attach right-click context menu to browser preview (D-09)
        {
            let menu_model = crate::menus::build_browser_context_menu();
            let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
            popover.set_parent(&widgets.container);
            popover.set_has_arrow(false);

            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // Right-click only
            gesture.connect_released({
                let popover = popover.clone();
                move |_, _, x, y| {
                    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                        x as i32, y as i32, 1, 1,
                    )));
                    popover.popup();
                }
            });
            widgets.container.add_controller(gesture);
        }

        let preview_node = SplitNode::Preview {
            pane_id: new_pane_id,
            container: widgets.container.clone(),
            picture: widgets.picture.clone(),
            url_entry: widgets.url_entry.clone(),
            uuid: widgets.uuid,
        };

        // Replace active leaf with Split(active_leaf, preview_node) -- vertical, preview on right
        let _replaced = self.replace_leaf_with_split(
            active_id,
            preview_node,
            gtk4::Orientation::Horizontal, // Horizontal paned = side-by-side (left terminal, right preview)
        )?;

        // Re-parent new root Paned into GtkStack if root was a single Leaf
        if let Some((stack, name)) = stack_slot {
            let new_root = self.root.widget();
            stack.add_named(&new_root, Some(&name));
            stack.set_visible_child_name(&name);
        }

        // Keep focus on the original terminal pane (do NOT change active_pane_id)
        // The terminal keeps keyboard input, preview is passive display only.
        self.root.update_focus_css(active_id);

        Some(widgets)
    }

    /// Replace the leaf with `target_pane_id` with a Split(orientation) node.
    /// Returns Some(()) on success, None if the leaf was not found.
    fn replace_leaf_with_split(
        &mut self,
        target_pane_id: u64,
        new_leaf: SplitNode,
        orientation: gtk4::Orientation,
    ) -> Option<()> {
        let orientation_cap = orientation;
        let mut replacer = Some(|old_leaf: SplitNode| {
            let old_widget = old_leaf.widget();
            let new_widget = new_leaf.widget();

            // GTK4 requires a widget to have no parent before set_start/end_child.
            // old_widget may be parented to the Stack (first split) or an outer Paned (nested).
            remove_widget_from_parent(&old_widget);

            let paned = gtk4::Paned::new(orientation_cap);
            // Both children must be allowed to resize — GTK4 default for resize_end_child
            // is TRUE but be explicit to ensure drag works in both directions.
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            // Prevent children from collapsing to 0px when dragging to an extreme.
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            // Wide handle makes the divider grabable (default is ~5px, hard to click).
            paned.set_wide_handle(true);

            paned.set_start_child(Some(&old_widget));
            paned.set_end_child(Some(&new_widget));

            // Set 50/50 position after the first layout pass (per D-09 and RESEARCH Pitfall 2).
            // connect_realize fires before GTK allocates sizes, so p.width() is 0 there.
            // idle_add_local_once defers to the next main-loop idle, after layout completes.
            {
                let paned_ref = paned.clone();
                gtk4::glib::idle_add_local_once(move || {
                    let size = if orientation_cap == gtk4::Orientation::Horizontal {
                        paned_ref.width()
                    } else {
                        paned_ref.height()
                    };
                    if size > 0 {
                        paned_ref.set_position(size / 2);
                    }
                });
            }

            // Restore GTK focus AND Ghostty focus after GtkPaned drag ends (Gap 1A).
            //
            // The divider drag temporarily moves GTK focus to the separator handle,
            // which causes Ghostty's cursor blink and keyboard input to stop.
            //
            // IMPORTANT: We must NOT restore focus on every `notify::position` change
            // during the drag — that fires on every pixel of movement and causes:
            //   1. grab_focus() thrashing that fights the active gesture
            //   2. ghostty_surface_set_focus(true) message storms to the render thread
            //   3. GL_AREA_REGISTRY lock contention with the resize idle handler
            // Instead, we detect the drag lifecycle via the paned's internal GestureDrag
            // controller and restore focus only once when the drag ends.
            {
                // Find the GtkPaned's internal GestureDrag on its separator handle.
                // GTK4's Paned uses a GestureDrag controller internally — we observe
                // the controller list and connect to its drag-end signal.
                //
                // The gesture lives on the separator handle widget, not the Paned
                // itself. Walk the Paned's children to find the separator, then
                // inspect its controllers.
                let mut found_gesture = false;

                // First try: controllers on the Paned itself
                let controllers = paned.observe_controllers();
                let n = controllers.n_items();
                for i in 0..n {
                    if let Some(obj) = controllers.item(i) {
                        if let Ok(gesture) = obj.downcast::<gtk4::GestureDrag>() {
                            gesture.connect_drag_end(|_gesture, _offset_x, _offset_y| {
                                // Defer to idle so GTK has time to fully release the gesture
                                // and clean up the event sequence before we move focus.
                                gtk4::glib::idle_add_local_once(|| {
                                    restore_active_pane_focus();
                                });
                            });
                            found_gesture = true;
                            break;
                        }
                    }
                }

                // Second try: walk children to find the separator handle widget
                if !found_gesture {
                    let mut child = paned.first_child();
                    while let Some(ref widget) = child {
                        let ctrl_list = widget.observe_controllers();
                        let cn = ctrl_list.n_items();
                        for i in 0..cn {
                            if let Some(obj) = ctrl_list.item(i) {
                                if let Ok(gesture) = obj.downcast::<gtk4::GestureDrag>() {
                                    gesture.connect_drag_end(|_gesture, _offset_x, _offset_y| {
                                        gtk4::glib::idle_add_local_once(|| {
                                            restore_active_pane_focus();
                                        });
                                    });
                                    found_gesture = true;
                                    break;
                                }
                            }
                        }
                        if found_gesture {
                            break;
                        }
                        child = widget.next_sibling();
                    }
                }

                if !found_gesture {
                    eprintln!("cmux: WARNING — no GestureDrag found on Paned or its children, falling back to notify::position");
                    // Fallback: use notify::position but debounced via idle.
                    // connect_notify requires Send+Sync, so use AtomicBool instead of Rc<Cell>.
                    let restore_pending =
                        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    paned.connect_notify(Some("position"), move |_paned, _pspec| {
                        if restore_pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            return;
                        }
                        let pending = restore_pending.clone();
                        gtk4::glib::idle_add_once(move || {
                            pending.store(false, std::sync::atomic::Ordering::SeqCst);
                            restore_active_pane_focus();
                        });
                    });
                }
            }

            SplitNode::Split {
                orientation: orientation_cap,
                paned: paned.clone(),
                start: Box::new(old_leaf),
                end: Box::new(new_leaf),
            }
        });
        replace_in_tree(&mut self.root, target_pane_id, &mut replacer)
    }

    /// Close one pane. If it was the active pane, focus moves to the surviving sibling.
    /// The pane's surface and GL state are torn down before its widget leaves the tree.
    pub fn close_pane(&mut self, target_id: u64) -> CloseResult {
        let target_is_terminal = match self.root.find_node(target_id) {
            Some(SplitNode::Leaf { .. }) => true,
            Some(SplitNode::Preview { .. }) => false,
            _ => return CloseResult::Failed,
        };
        // A root that is itself the pane is the workspace's only pane. The workspace closes instead.
        if !matches!(self.root, SplitNode::Split { .. }) {
            return CloseResult::LastPane;
        }
        // Never leave a workspace with only previews: the workspace closes instead.
        if would_close_last_terminal(count_terminals(&self.root), target_is_terminal) {
            return CloseResult::LastPane;
        }

        // Tear down the surface and GL state while the widget is still in the tree.
        if let Some(SplitNode::Leaf { gl_area, surface, ctx, .. }) = self.root.find_node(target_id) {
            teardown_pane(gl_area, surface, *ctx);
        }

        // Remove the leaf from the tree and get the surviving sibling's pane_id.
        let Some(surviving_id) = remove_leaf_from_tree(&mut self.root, target_id) else {
            return CloseResult::Failed;
        };

        if self.active_pane_id != target_id {
            // Closing a background pane leaves focus where it was.
            return CloseResult::Closed(self.active_pane_id);
        }

        // Update focus to the surviving pane.
        self.active_pane_id = surviving_id;
        self.root.update_focus_css(surviving_id);

        // Call ghostty_surface_set_focus on the surviving surface (SPLIT-07).
        if let Some(surface) = self.find_surface(surviving_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(surface, true);
            }
        }

        // Grab GTK focus on the surviving pane's widget (GLArea or URL entry).
        if let Some(gl_area) = self.find_gl_area(surviving_id) {
            gl_area.grab_focus();
        } else if let Some(entry) = find_url_entry_in_tree(&self.root, surviving_id) {
            entry.grab_focus();
        }

        CloseResult::Closed(surviving_id)
    }

    /// Compatibility wrapper for callers written before `CloseResult`: `Some(new active pane)`
    /// when the active pane closed, `None` for the last pane or a failure. Prefer `close_pane`,
    /// which tells those two apart.
    pub fn close_active(&mut self) -> Option<u64> {
        match self.close_pane(self.active_pane_id) {
            CloseResult::Closed(id) => Some(id),
            CloseResult::LastPane | CloseResult::Failed => None,
        }
    }

    /// Tears down every pane's surface and GL state. Call before the workspace's widgets are
    /// removed. The engine must not render or focus anything afterwards.
    pub fn teardown_all(&self) {
        teardown_leaves(&self.root);
    }

    /// Returns true if `pane_id` is a pane (terminal or preview) in this workspace.
    pub fn contains_pane(&self, pane_id: u64) -> bool {
        self.root.find_node(pane_id).is_some()
    }

    /// Navigate focus to the pane adjacent in `direction` (Ctrl+Alt+arrows per D-10).
    pub fn focus_next_in_direction(&mut self, direction: FocusDirection) -> bool {
        let active_id = self.active_pane_id;
        if let Some(new_id) = find_adjacent(&self.root, active_id, direction) {
            // Unfocus old surface.
            if let Some(old_surface) = self.find_surface(active_id) {
                unsafe {
                    ffi::ghostty_surface_set_focus(old_surface, false);
                }
            }
            self.active_pane_id = new_id;
            self.root.update_focus_css(new_id);
            // Focus new surface or Preview URL entry.
            if let Some(new_surface) = self.find_surface(new_id) {
                unsafe {
                    ffi::ghostty_surface_set_focus(new_surface, true);
                }
            }
            if let Some(gl_area) = self.find_gl_area(new_id) {
                gl_area.grab_focus();
            } else if let Some(entry) = find_url_entry_in_tree(&self.root, new_id) {
                entry.grab_focus();
            }
            true
        } else {
            false
        }
    }

    /// The pane's live surface, read through its handle. None before realize and after teardown.
    fn find_surface(&self, pane_id: u64) -> Option<ffi::ghostty_surface_t> {
        find_surface_in_tree(&self.root, pane_id)
    }

    fn find_gl_area(&self, pane_id: u64) -> Option<gtk4::GLArea> {
        find_gl_area_in_tree(&self.root, pane_id)
    }

    /// Returns all leaf panes in this engine as (uuid, pane_id, active) tuples.
    pub fn all_panes(&self) -> Vec<(Uuid, u64, bool)> {
        let mut panes = Vec::new();
        self.root.collect_pane_info(&mut panes, self.active_pane_id);
        panes
    }

    /// Look up a surface by its UUID string. Returns the ghostty surface handle if found.
    pub fn find_surface_by_uuid(&self, target_uuid: &str) -> Option<ffi::ghostty_surface_t> {
        self.root.find_by_uuid(target_uuid)
    }

    /// Look up a pane_id by its UUID string.
    pub fn find_pane_id_by_uuid(&self, target_uuid: &str) -> Option<u64> {
        self.root.find_pane_id_by_uuid(target_uuid)
    }

    /// Look up a GLArea by pane_id (public wrapper for socket handlers).
    pub fn gl_area_for_pane(&self, pane_id: u64) -> Option<gtk4::GLArea> {
        find_gl_area_in_tree(&self.root, pane_id)
    }
}

// ── Tree traversal helpers ───────────────────────────────────────────────────

/// Replace the leaf with `target_id` using `replacer` function. Returns Some(()) if found.
fn replace_in_tree<F>(node: &mut SplitNode, target_id: u64, replacer: &mut Option<F>) -> Option<()>
where
    F: FnOnce(SplitNode) -> SplitNode,
{
    match node {
        SplitNode::Leaf { pane_id, .. } if *pane_id == target_id => {
            if let Some(r) = replacer.take() {
                // Take ownership of the old node to pass to replacer.
                let old = std::mem::replace(
                    node,
                    SplitNode::Leaf {
                        pane_id: 0,
                        gl_area: gtk4::GLArea::new(),
                        surface: Rc::new(RefCell::new(None)),
                        ctx: std::ptr::null_mut(),
                        uuid: Uuid::new_v4(),
                        has_attention: false,
                    },
                );
                *node = r(old);
                Some(())
            } else {
                None
            }
        }
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // Cannot split a preview pane
        SplitNode::Split {
            start, end, paned, ..
        } => {
            if let Some(()) = replace_in_tree(start, target_id, replacer) {
                // Update paned start child to new widget.
                paned.set_start_child(Some(&start.widget()));
                Some(())
            } else if let Some(()) = replace_in_tree(end, target_id, replacer) {
                paned.set_end_child(Some(&end.widget()));
                Some(())
            } else {
                None
            }
        }
    }
}

/// Remove leaf `target_id` from the tree. Returns the surviving sibling's pane_id.
/// Replaces the parent Split with the surviving sibling in the GTK widget tree.
fn remove_leaf_from_tree(node: &mut SplitNode, target_id: u64) -> Option<u64> {
    match node {
        SplitNode::Leaf { .. } | SplitNode::Preview { .. } => None, // Caller ensures we never remove the root leaf
        SplitNode::Split {
            start, end, paned, ..
        } => {
            // Check if start is the target (Leaf or Preview).
            let start_is_target = match start.as_ref() {
                SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id == target_id,
                _ => false,
            };
            if start_is_target {
                // Surviving sibling is end. Replace this Split with end in the GTK tree.
                let surviving = *end.clone();
                let surviving_widget = surviving.widget();
                // Find the paned's parent and replace it with the surviving widget.
                if let Some(parent) = paned.parent() {
                    replace_child_in_parent(
                        &parent,
                        &paned.clone().upcast(),
                        &surviving_widget,
                    );
                }
                let surviving_id = first_pane_id(&surviving);
                *node = surviving;
                return Some(surviving_id);
            }
            // Check if end is the target (Leaf or Preview).
            let end_is_target = match end.as_ref() {
                SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id == target_id,
                _ => false,
            };
            if end_is_target {
                let surviving = *start.clone();
                let surviving_widget = surviving.widget();
                if let Some(parent) = paned.parent() {
                    replace_child_in_parent(
                        &parent,
                        &paned.clone().upcast(),
                        &surviving_widget,
                    );
                }
                let surviving_id = first_pane_id(&surviving);
                *node = surviving;
                return Some(surviving_id);
            }
            // Recurse into start subtree.
            if let Some(id) = remove_leaf_from_tree(start, target_id) {
                paned.set_start_child(Some(&start.widget()));
                return Some(id);
            }
            // Recurse into end subtree.
            if let Some(id) = remove_leaf_from_tree(end, target_id) {
                paned.set_end_child(Some(&end.widget()));
                return Some(id);
            }
            None
        }
    }
}

/// Replace `old_widget` with `new_widget` in `parent`. Handles GtkPaned children and GtkStack pages.
fn replace_child_in_parent(
    parent: &gtk4::Widget,
    old_widget: &gtk4::Widget,
    new_widget: &gtk4::Widget,
) {
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        // new_widget is still parented to the Paned being replaced; unparent it first.
        remove_widget_from_parent(new_widget);
        if paned
            .start_child()
            .as_ref()
            .map(|w| w == old_widget)
            .unwrap_or(false)
        {
            paned.set_start_child(Some(new_widget));
        } else {
            paned.set_end_child(Some(new_widget));
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        let page = stack.page(old_widget);
        if let Some(name) = page.name() {
            let name_str = name.to_string();
            stack.remove(old_widget);
            // new_widget may still be parented to the Paned we're replacing; unparent first.
            remove_widget_from_parent(new_widget);
            stack.add_named(new_widget, Some(&name_str));
            stack.set_visible_child_name(&name_str);
        } else {
            stack.remove(old_widget);
        }
    }
    // If parent is something else, the widget swap is a no-op (should not happen in Phase 2).
}

/// Return the first (leftmost/topmost) pane_id in a subtree.
fn first_pane_id(node: &SplitNode) -> u64 {
    match node {
        SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id,
        SplitNode::Split { start, .. } => first_pane_id(start),
    }
}

fn find_surface_in_tree(node: &SplitNode, pane_id: u64) -> Option<ffi::ghostty_surface_t> {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            surface,
            ..
        } if *id == pane_id => *surface.borrow(),
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // No Ghostty surface
        SplitNode::Split { start, end, .. } => {
            find_surface_in_tree(start, pane_id).or_else(|| find_surface_in_tree(end, pane_id))
        }
    }
}

fn find_gl_area_in_tree(node: &SplitNode, pane_id: u64) -> Option<gtk4::GLArea> {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            gl_area,
            ..
        } if *id == pane_id => Some(gl_area.clone()),
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // Preview uses Picture, not GLArea
        SplitNode::Split { start, end, .. } => {
            find_gl_area_in_tree(start, pane_id).or_else(|| find_gl_area_in_tree(end, pane_id))
        }
    }
}

/// Count terminal (Leaf) panes in the tree.
fn count_terminals(node: &SplitNode) -> usize {
    match node {
        SplitNode::Leaf { .. } => 1,
        SplitNode::Preview { .. } => 0,
        SplitNode::Split { start, end, .. } => count_terminals(start) + count_terminals(end),
    }
}

/// Tears down every leaf's surface and GL state in a subtree. Each leaf is torn down once.
fn teardown_leaves(node: &SplitNode) {
    match node {
        SplitNode::Leaf { gl_area, surface, ctx, .. } => teardown_pane(gl_area, surface, *ctx),
        SplitNode::Preview { .. } => {}
        SplitNode::Split { start, end, .. } => {
            teardown_leaves(start);
            teardown_leaves(end);
        }
    }
}

/// Closing a pane is refused when it is the workspace's last terminal: the workspace must
/// close instead, so a workspace is never left with only preview panes.
fn would_close_last_terminal(terminal_count: usize, target_is_terminal: bool) -> bool {
    target_is_terminal && terminal_count <= 1
}

fn find_url_entry_in_tree(node: &SplitNode, pane_id: u64) -> Option<gtk4::Entry> {
    match node {
        SplitNode::Preview { pane_id: id, url_entry, .. } if *id == pane_id => Some(url_entry.clone()),
        SplitNode::Preview { .. } | SplitNode::Leaf { .. } => None,
        SplitNode::Split { start, end, .. } => {
            find_url_entry_in_tree(start, pane_id).or_else(|| find_url_entry_in_tree(end, pane_id))
        }
    }
}

/// Find the pane adjacent to `active_id` in `direction`.
/// Strategy: collect ordered leaf positions and find the neighbor.
/// This is a directional approximation: Left/Up = previous leaf, Right/Down = next leaf.
/// A full spatial algorithm (comparing widget coordinates) can be added in a future phase.
fn find_adjacent(root: &SplitNode, active_id: u64, direction: FocusDirection) -> Option<u64> {
    let mut leaves = Vec::new();
    collect_leaves_in_order(root, &mut leaves);
    let pos = leaves.iter().position(|&id| id == active_id)?;
    match direction {
        FocusDirection::Left | FocusDirection::Up => {
            if pos > 0 {
                Some(leaves[pos - 1])
            } else {
                None
            }
        }
        FocusDirection::Right | FocusDirection::Down => {
            if pos + 1 < leaves.len() {
                Some(leaves[pos + 1])
            } else {
                None
            }
        }
    }
}

/// Remove `widget` from its current GTK parent so it can be reparented.
/// GTK4 requires `gtk_widget_get_parent(child) == NULL` before set_start/end_child.
fn remove_widget_from_parent(widget: &gtk4::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        if paned
            .start_child()
            .as_ref()
            .map(|w| w == widget)
            .unwrap_or(false)
        {
            paned.set_start_child(None::<&gtk4::Widget>);
        } else {
            paned.set_end_child(None::<&gtk4::Widget>);
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        stack.remove(widget);
    }
}

/// Restore GTK keyboard focus and Ghostty surface focus to the active pane.
/// Called once when a GtkPaned drag ends — NOT on every pixel of movement.
/// Re-syncs each surface's cached size with the GLArea's current allocation to
/// break any anti-flicker stall in Ghostty's drawFrame() guard, then kicks
/// the render thread with ghostty_surface_refresh + queue_render.
///
/// Does NOT touch focus state. The cursor blink timer runs independently of
/// resize. Calling set_focus(false→true) here kills the timer due to an async
/// cancel race in Ghostty's renderer thread: the false message enqueues a timer
/// cancel, but the true message is processed before the cancel callback fires,
/// so the guard `if cursor_c.state() != .active` sees `.active` and skips the
/// restart. The cancel then completes, leaving the timer permanently dead.
fn restore_active_pane_focus() {
    // Re-set size + refresh ALL surfaces to break the anti-flicker stall.
    if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
        if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if let Some(&surface_ptr) = gl_to_surface.get(&(area_ptr.0 as usize)) {
                    let scale = area.scale_factor();
                    let w = (area.width() * scale) as u32;
                    let h = (area.height() * scale) as u32;
                    if w > 0 && h > 0 {
                        unsafe {
                            let surface = surface_ptr as ffi::ghostty_surface_t;
                            ffi::ghostty_surface_set_size(surface, w, h);
                            ffi::ghostty_surface_refresh(surface);
                        }
                    }
                }
                if area.is_realized() {
                    area.queue_render();
                    area.queue_draw();
                }
            }
        }
    }

    // Drive the app tick to process any pending mailbox messages (redraw_surface, etc.)
    let app_ptr = crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
    if app_ptr != 0 {
        unsafe {
            let app = app_ptr as ffi::ghostty_app_t;
            ffi::ghostty_app_tick(app);
        }
    }

    // Restore GTK keyboard focus to the active pane.
    if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
        for area_ptr in areas.iter() {
            let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
            if area.has_css_class("active-pane") {
                area.grab_focus();
                area.queue_render();
                break;
            }
        }
    }

    // The set_size → IO thread → render thread → updateFrame → cells rebuild pipeline
    // is asynchronous. The immediate queue_render above may still draw stale content
    // because cells haven't been rebuilt yet. Schedule follow-up recovery ticks to
    // give the pipeline time to converge (50ms, 150ms, 300ms).
    for delay_ms in [50u32, 150, 300] {
        gtk4::glib::timeout_add_local_once(
            std::time::Duration::from_millis(delay_ms as u64),
            move || {
                let app_ptr =
                    crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
                if app_ptr != 0 {
                    unsafe {
                        ffi::ghostty_app_tick(app_ptr as ffi::ghostty_app_t);
                    }
                }
                if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                    for area_ptr in areas.iter() {
                        let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                            unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                        if area.is_realized() {
                            area.queue_render();
                        }
                    }
                }
            },
        );
    }
}

fn collect_leaves_in_order(node: &SplitNode, out: &mut Vec<u64>) {
    match node {
        SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => out.push(*pane_id),
        SplitNode::Split { start, end, .. } => {
            collect_leaves_in_order(start, out);
            collect_leaves_in_order(end, out);
        }
    }
}

/// Serde-friendly mirror of SplitNode for session persistence.
/// GTK widget references (GLArea, Paned) cannot be serialized — this parallel type holds
/// only the data needed to reconstruct the tree on restore.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SplitNodeData {
    Leaf {
        pane_id: u64,
        surface_uuid: Uuid,
        /// Shell executable path, e.g. "/bin/zsh" or "/bin/bash"
        shell: String,
        /// Absolute working directory path (best-effort; may be empty if /proc unavailable)
        cwd: String,
    },
    Split {
        /// "horizontal" or "vertical"
        orientation: String,
        /// Divider position as fraction 0.0-1.0 relative to parent size (D-03).
        #[serde(default = "default_ratio")]
        ratio: f64,
        start: Box<SplitNodeData>,
        end: Box<SplitNodeData>,
    },
}

fn default_ratio() -> f64 {
    0.5
}

/// Combines the session data of a split's two children. A child that is missing (a preview
/// pane, which is not saved) collapses the split into the other child.
fn combine_split(
    orientation: String,
    ratio: f64,
    start: Option<SplitNodeData>,
    end: Option<SplitNodeData>,
) -> Option<SplitNodeData> {
    match (start, end) {
        (Some(start), Some(end)) => Some(SplitNodeData::Split {
            orientation,
            ratio,
            start: Box::new(start),
            end: Box::new(end),
        }),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

/// Best-effort CWD capture for a Ghostty surface via /proc.
/// Walks /proc looking for child processes of the current process (cmux),
/// then reads /proc/{pid}/cwd for the foreground shell.
/// Never panics — falls back to $HOME or empty string.
fn get_surface_cwd(surface: Option<ffi::ghostty_surface_t>) -> String {
    if surface.is_none() {
        return String::new();
    }
    // Try to find child shell processes by scanning /proc for children of our PID.
    // Each Ghostty surface spawns a child shell — we look for processes whose
    // parent is the cmux process and read their CWD.
    let our_pid = std::process::id();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        // Collect candidate child PIDs (children of our process)
        let mut candidates: Vec<u32> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<u32>() {
                // Read /proc/{pid}/stat to check parent PID
                if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                    // Format: pid (comm) state ppid ...
                    // Find the closing paren then parse ppid
                    if let Some(after_comm) = stat.rfind(')') {
                        let fields: Vec<&str> = stat[after_comm + 2..].split_whitespace().collect();
                        if fields.len() >= 2 {
                            if let Ok(ppid) = fields[1].parse::<u32>() {
                                if ppid == our_pid {
                                    candidates.push(pid);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Use the last (most recent) child process CWD as best guess.
        // In practice, each surface has one direct child shell.
        for pid in candidates.iter().rev() {
            if let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
                let cwd_str = cwd.to_string_lossy().to_string();
                if !cwd_str.is_empty() {
                    return cwd_str;
                }
            }
        }
    }
    // Fallback to $HOME
    std::env::var("HOME").unwrap_or_default()
}

impl SplitNode {
    /// Session snapshot of this subtree. Preview panes are ephemeral and are skipped; a split
    /// left with one child collapses to that child. Returns None when no terminal is left.
    pub fn to_session_data(&self) -> Option<SplitNodeData> {
        match self {
            SplitNode::Leaf { pane_id, uuid, surface, .. } => {
                let cwd = get_surface_cwd(*surface.borrow());
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                Some(SplitNodeData::Leaf {
                    pane_id: *pane_id,
                    surface_uuid: *uuid,
                    shell,
                    cwd,
                })
            }
            SplitNode::Preview { .. } => None,
            SplitNode::Split { orientation, paned, start, end, .. } => {
                let total_size = if *orientation == gtk4::Orientation::Horizontal {
                    paned.width()
                } else {
                    paned.height()
                };
                let ratio = if total_size > 0 {
                    (paned.position() as f64) / (total_size as f64)
                } else {
                    0.5 // default if not yet laid out
                };
                let orientation = match orientation {
                    gtk4::Orientation::Horizontal => "horizontal".to_string(),
                    gtk4::Orientation::Vertical => "vertical".to_string(),
                    _ => "horizontal".to_string(),
                };
                combine_split(
                    orientation,
                    ratio,
                    start.to_session_data(),
                    end.to_session_data(),
                )
            }
        }
    }

    /// Serialized tree, with a placeholder leaf if no terminal is left. A workspace always keeps
    /// at least one terminal, so the placeholder does not occur in practice. Prefer
    /// `to_session_data` where a missing value can be handled.
    pub fn to_data(&self) -> SplitNodeData {
        self.to_session_data().unwrap_or_else(|| SplitNodeData::Leaf {
            pane_id: 0,
            surface_uuid: Uuid::nil(),
            shell: String::new(),
            cwd: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_data(pane_id: u64) -> SplitNodeData {
        SplitNodeData::Leaf {
            pane_id,
            surface_uuid: Uuid::new_v4(),
            shell: String::new(),
            cwd: String::new(),
        }
    }

    #[test]
    fn split_node_data_leaf_has_surface_uuid() {
        // Build a minimal SplitNodeData::Leaf directly and verify surface_uuid field exists.
        let id = Uuid::new_v4();
        let data = SplitNodeData::Leaf {
            pane_id: 42,
            surface_uuid: id,
            shell: "/bin/bash".to_string(),
            cwd: "/home/user".to_string(),
        };
        if let SplitNodeData::Leaf { surface_uuid, pane_id, .. } = data {
            assert_eq!(surface_uuid, id);
            assert_eq!(pane_id, 42);
        } else {
            panic!("Expected SplitNodeData::Leaf");
        }
    }

    #[test]
    fn split_node_data_roundtrip_json() {
        // Verify SplitNodeData serializes and deserializes via serde_json.
        let leaf = SplitNodeData::Leaf {
            pane_id: 1,
            surface_uuid: Uuid::new_v4(),
            shell: "/bin/zsh".to_string(),
            cwd: "/tmp".to_string(),
        };
        let json = serde_json::to_string(&leaf).expect("serialize failed");
        let restored: SplitNodeData = serde_json::from_str(&json).expect("deserialize failed");
        if let (
            SplitNodeData::Leaf { pane_id: p1, surface_uuid: u1, .. },
            SplitNodeData::Leaf { pane_id: p2, surface_uuid: u2, .. },
        ) = (&leaf, &restored)
        {
            assert_eq!(p1, p2);
            assert_eq!(u1, u2);
        } else {
            panic!("Roundtrip changed variant");
        }
    }

    #[test]
    fn split_node_data_split_roundtrip_json() {
        // Verify nested SplitNodeData serializes correctly with ratio field.
        let split = SplitNodeData::Split {
            orientation: "horizontal".to_string(),
            ratio: 0.35,
            start: Box::new(SplitNodeData::Leaf {
                pane_id: 1,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
            }),
            end: Box::new(SplitNodeData::Leaf {
                pane_id: 2,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
            }),
        };
        let json = serde_json::to_string(&split).expect("serialize failed");
        let restored: SplitNodeData = serde_json::from_str(&json).expect("deserialize failed");
        if let SplitNodeData::Split { orientation, ratio, .. } = restored {
            assert_eq!(orientation, "horizontal");
            assert!((ratio - 0.35).abs() < f64::EPSILON, "ratio not preserved in roundtrip");
        } else {
            panic!("Roundtrip changed variant to non-Split");
        }

        // Verify v1-compat: Split without ratio field deserializes with default 0.5
        let v1_json = r#"{"type":"Split","orientation":"vertical","start":{"type":"Leaf","pane_id":1,"surface_uuid":"00000000-0000-0000-0000-000000000000","shell":"","cwd":""},"end":{"type":"Leaf","pane_id":2,"surface_uuid":"00000000-0000-0000-0000-000000000000","shell":"","cwd":""}}"#;
        let v1_restored: SplitNodeData = serde_json::from_str(v1_json).expect("v1 deserialize failed");
        if let SplitNodeData::Split { ratio, .. } = v1_restored {
            assert!((ratio - 0.5).abs() < f64::EPSILON, "v1 missing ratio should default to 0.5");
        } else {
            panic!("v1 deserialize changed variant");
        }
    }

    #[test]
    fn pane_ids_are_unique() {
        let ids: std::collections::HashSet<u64> = (0..1000).map(|_| alloc_pane_id()).collect();
        assert_eq!(ids.len(), 1000, "pane id allocation returned a duplicate");
        assert!(!ids.contains(&0), "0 is reserved for 'no pane'");
    }

    #[test]
    fn split_with_missing_child_collapses_to_other_child() {
        let collapsed = combine_split("horizontal".into(), 0.5, Some(leaf_data(1)), None);
        assert!(matches!(collapsed, Some(SplitNodeData::Leaf { pane_id: 1, .. })));

        let collapsed = combine_split("horizontal".into(), 0.5, None, Some(leaf_data(2)));
        assert!(matches!(collapsed, Some(SplitNodeData::Leaf { pane_id: 2, .. })));

        assert!(combine_split("horizontal".into(), 0.5, None, None).is_none());

        match combine_split("vertical".into(), 0.25, Some(leaf_data(1)), Some(leaf_data(2))) {
            Some(SplitNodeData::Split { orientation, ratio, start, end }) => {
                assert_eq!(orientation, "vertical");
                assert!((ratio - 0.25).abs() < f64::EPSILON);
                assert!(matches!(*start, SplitNodeData::Leaf { pane_id: 1, .. }));
                assert!(matches!(*end, SplitNodeData::Leaf { pane_id: 2, .. }));
            }
            other => panic!("expected a split, got {other:?}"),
        }
    }

    #[test]
    fn last_terminal_close_is_refused() {
        // Closing the only terminal must hand off to the workspace close.
        assert!(would_close_last_terminal(1, true));
        // Two terminals: closing one is fine.
        assert!(!would_close_last_terminal(2, true));
        // Closing a preview while one terminal remains is fine.
        assert!(!would_close_last_terminal(1, false));
        assert!(!would_close_last_terminal(0, false));
    }
}
