use crate::ghostty::ffi;
use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// I/O mode for a Ghostty surface.
/// - `Exec`: normal mode — Ghostty spawns a local shell process.
/// - `Manual`: SSH remote mode — keystrokes route through io_write_cb to the SSH bridge.
#[derive(Clone)]
pub enum SurfaceIoMode {
    Exec,
    Manual {
        io_write_ctx: std::sync::Arc<crate::ssh::bridge::IoWriteContext>,
    },
}

/// Shared handle to a pane's live Ghostty surface. `None` until the GLArea realize
/// callback creates the surface, and again after teardown. Every read of a pane's
/// surface goes through this cell, so a freed surface is never reachable from the tree.
pub type SurfaceCell = Rc<RefCell<Option<ffi::ghostty_surface_t>>>;

/// Per-pane context handed to Ghostty as surface userdata. Allocated once per pane in
/// `create_surface`; freed exactly once by `teardown_pane`, after the surface is freed.
pub struct PaneCtx {
    pub pane_id: u64,
    pub surface: SurfaceCell,
    /// Weak, so the context never keeps the GLArea alive.
    pub gl_area: glib::WeakRef<gtk4::GLArea>,
}

/// Owned handles for one new pane, returned by `create_surface`.
pub struct PaneSurface {
    pub gl_area: gtk4::GLArea,
    pub surface: SurfaceCell,
    /// Heap `PaneCtx` owned by the pane. Freed by `teardown_pane`.
    pub ctx: *mut PaneCtx,
}

/// Empty NUL-terminated string, used to deny a clipboard request.
const EMPTY_C_STR: &[u8] = b"\0";

/// Borrows the pane context from Ghostty's userdata pointer.
///
/// # Safety
/// `userdata` must be null or a `PaneCtx` pointer from `create_surface` that has not
/// been freed by `teardown_pane`.
unsafe fn pane_ctx<'a>(userdata: *mut std::ffi::c_void) -> Option<&'a PaneCtx> {
    (userdata as *const PaneCtx).as_ref()
}

/// Call glGetError() via FFI to check for GL errors after ghostty calls.
/// Returns 0 if no error, or the GL error code otherwise.
fn gl_get_error() -> u32 {
    extern "C" {
        fn glGetError() -> u32;
    }
    unsafe { glGetError() }
}

/// Releases one pane's GL and surface state. Must run before the pane's GLArea is removed
/// from the widget tree, and exactly once per pane (it frees `ctx`).
///
/// Order: deregister the GLArea and take the surface out of its cell, so no callback can
/// reach either; unrealize GL resources while the context is current; free the surface;
/// free the context.
pub fn teardown_pane(gl_area: &gtk4::GLArea, surface: &SurfaceCell, ctx: *mut PaneCtx) {
    use crate::ghostty::callbacks::{GL_AREA_REGISTRY, GL_TO_SURFACE, SURFACE_REGISTRY};
    use gtk4::prelude::*;

    let area_key = gl_area.as_ptr() as usize;
    if let Ok(mut areas) = GL_AREA_REGISTRY.lock() {
        areas.retain(|p| p.0 as usize != area_key);
    }
    if let Ok(mut map) = GL_TO_SURFACE.lock() {
        map.remove(&area_key);
    }

    let taken = surface.borrow_mut().take();
    if let Some(s) = taken {
        if let Ok(mut reg) = SURFACE_REGISTRY.lock() {
            reg.remove(&(s as usize));
        }
        // A realized GLArea still owns GL objects. An unrealized one was already released
        // by the unrealize callback.
        if gl_area.is_realized() {
            gl_area.make_current();
            unsafe {
                ffi::ghostty_surface_display_unrealized(s);
            }
        }
        unsafe {
            ffi::ghostty_surface_free(s);
        }
    }

    if !ctx.is_null() {
        unsafe {
            drop(Box::from_raw(ctx));
        }
    }
}

/// Creates and returns a GtkGLArea with a Ghostty terminal surface wired up.
/// Initializes ghostty_app_t, then defers ghostty_surface_t creation to the
/// GtkGLArea realize signal — when the GL context is guaranteed to exist.
/// The returned `PaneSurface` owns the pane's context; release it with `teardown_pane`.
pub fn create_surface(
    _app: &gtk4::Application,
    ghostty_app: ffi::ghostty_app_t,
    inherited_config: Option<ffi::ghostty_surface_config_s>,
    pane_id: u64,
    io_mode: SurfaceIoMode,
) -> PaneSurface {
    use gtk4::prelude::*;

    use crate::ghostty::callbacks;

    eprintln!(
        "cmux: create_surface called for pane_id={}, inherited_config={}",
        pane_id,
        inherited_config.is_some()
    );

    let gl_area = gtk4::GLArea::new();
    // Per Pitfall 1: require OpenGL 4.3 before the area is realized.
    gl_area.set_required_version(4, 3);
    // Manual render mode: only render when wakeup_cb schedules queue_render().
    // An independent render loop adds input latency (per CLAUDE.md pitfall).
    gl_area.set_auto_render(false);
    // Must be focusable to receive keyboard events via EventControllerKey.
    gl_area.set_focusable(true);
    // Grab keyboard focus when the user clicks inside the terminal.
    gl_area.set_focus_on_click(true);
    // Expand to fill available space — required for GtkPaned to distribute space evenly.
    // Without this, GLArea has natural size 0 and the Paned gives all space to end child.
    gl_area.set_hexpand(true);
    gl_area.set_vexpand(true);

    // Shared cell for the surface pointer — created in realize (after GL context exists),
    // then used in render, resize, input, and scale-factor callbacks.
    // Rc<RefCell<...>> is safe here: all callbacks run on the GLib main thread.
    let surface_cell: SurfaceCell = Rc::new(RefCell::new(None));

    // Per-pane context: Ghostty passes it back as userdata to clipboard and close callbacks.
    // Freed by teardown_pane, never by Drop.
    let ctx: *mut PaneCtx = Box::into_raw(Box::new(PaneCtx {
        pane_id,
        surface: surface_cell.clone(),
        gl_area: gl_area.downgrade(),
    }));

    // ── GtkGLArea::realize ───────────────────────────────────────────────────
    // GL context is now valid. Create the surface HERE so ghostty can access
    // the GL context immediately after creation (fixes segfault in set_content_scale).
    //
    // IMPORTANT: GTK may re-realize the widget when reparenting (e.g., moving from
    // GtkStack into GtkPaned during split). We must check if the surface already
    // exists and reuse it, otherwise we create orphaned surfaces that never render.
    gl_area.connect_realize({
        let cell = surface_cell.clone();
        let io_mode = io_mode;
        move |area| {
            area.make_current();
            if let Some(err) = area.error() {
                eprintln!("cmux: GLArea realize error: {err}");
                std::process::exit(1); // Per D-09: no GUI error dialog in Phase 1
            }

            // Check if surface already exists (re-realize after reparent).
            // If so, just update the size/scale and refresh — don't create a new surface.
            //
            // DO NOT restore focus here. During a split, the old pane is reparented into
            // a GtkPaned — it should NOT regain focus. The new pane gets focus via its own
            // fresh realize path (set_focus(true) + grab_focus). EventControllerFocus
            // handles focus restoration automatically when GTK gives this widget focus back
            // (via `enter` signal). Calling set_focus(true) here incorrectly marks the old
            // pane as focused, causing both panes to have focused=true simultaneously and
            // triggering Ghostty's early-return guard on the new pane's subsequent focus calls.
            if let Some(existing_surface) = *cell.borrow() {
                // Reinitialize renderer GL resources (shaders, swap chain) for the
                // new GL context. This matches Ghostty's own GTK apprt glareaRealize
                // which calls displayRealized() after reparent/display-move.
                unsafe {
                    ffi::ghostty_surface_display_realized(existing_surface);
                }
                let scale = area.scale_factor() as f64;
                let w = area.width();
                let h = area.height();
                unsafe {
                    let phys_w = (w as f64 * scale) as u32;
                    let phys_h = (h as f64 * scale) as u32;
                    if phys_w > 0 && phys_h > 0 {
                        ffi::ghostty_surface_set_size(existing_surface, phys_w, phys_h);
                    }
                    ffi::ghostty_surface_set_content_scale(existing_surface, scale, scale);
                    ffi::ghostty_surface_refresh(existing_surface);
                }
                area.queue_render();
                return;
            }

            // Log GL version and renderer info for diagnostics.
            if let Some(gl_ctx) = area.context() {
                let (major, minor) = gl_ctx.version();
                eprintln!("cmux: GL context version: {major}.{minor}");
            }

            // Create the ghostty surface now that GL context is current.
            let surface = unsafe {
                let gl_area_ptr = area.as_ptr() as *mut std::ffi::c_void;

                let platform = ffi::ghostty_platform_u {
                    gtk4: ffi::ghostty_platform_gtk4_s {
                        gl_area: gl_area_ptr,
                    },
                };

                let mut surface_config = if let Some(ic) = inherited_config {
                    ic // already owned by value — no dangling pointer
                } else {
                    ffi::ghostty_surface_config_new()
                };

                surface_config.platform_tag = ffi::ghostty_platform_e_GHOSTTY_PLATFORM_GTK4;
                surface_config.platform = platform;
                // Surface callbacks (clipboard, close) receive this pointer as userdata.
                surface_config.userdata = ctx as *mut std::ffi::c_void;
                surface_config.scale_factor = area.scale_factor() as f64;

                // Set manual I/O mode for SSH remote surfaces.
                if let SurfaceIoMode::Manual { ref io_write_ctx } = io_mode {
                    surface_config.io_mode = ffi::ghostty_surface_io_mode_e_GHOSTTY_SURFACE_IO_MANUAL;
                    surface_config.io_write_cb = Some(crate::ssh::bridge::ssh_io_write_cb);
                    // Increment Arc refcount for the raw pointer — the matching
                    // Arc::from_raw happens in unrealize/cleanup.
                    let ctx_raw = std::sync::Arc::into_raw(io_write_ctx.clone()) as *mut std::ffi::c_void;
                    surface_config.io_write_userdata = ctx_raw;
                }

                let s = ffi::ghostty_surface_new(ghostty_app, &surface_config);
                if s.is_null() {
                    eprintln!("cmux: FATAL — ghostty_surface_new returned null");
                    std::process::exit(1);
                }
                // Check GL error state after surface creation.
                let gl_err = gl_get_error();
                if gl_err != 0 {
                    eprintln!("cmux: GL error after ghostty_surface_new: 0x{gl_err:x}");
                }
                s
            };

            if let Ok(mut registry) = callbacks::SURFACE_REGISTRY.lock() {
                registry.insert(surface as usize, pane_id);
            }

            // Set initial size and scale after surface creation.
            let scale = area.scale_factor() as f64;
            let w = area.width();
            let h = area.height();
            unsafe {
                // Per Pitfall 5: convert logical→physical pixels.
                // Guard against calling set_size(0,0) at realize time — widget has not been
                // allocated its real size yet. connect_resize will fire with the correct size.
                let phys_w = (w as f64 * scale) as u32;
                let phys_h = (h as f64 * scale) as u32;
                if phys_w > 0 && phys_h > 0 {
                    ffi::ghostty_surface_set_size(surface, phys_w, phys_h);
                }
                ffi::ghostty_surface_set_content_scale(surface, scale, scale);
                ffi::ghostty_surface_set_focus(surface, true);
            }

            // Store the surface pointer BEFORE grab_focus so that EventControllerFocus
            // `enter` can call set_focus(true) when the widget receives GTK keyboard focus.
            // If we store after, the enter handler finds cell=None and is a no-op.
            *cell.borrow_mut() = Some(surface);

            // Grab GTK keyboard focus so the terminal receives key events immediately
            // without requiring a mouse click (belt-and-suspenders with switch_to_index).
            area.grab_focus();

            // Register this GLArea in the multi-surface registry for wakeup_cb.
            // teardown_pane removes it again.
            if let Ok(mut areas) = callbacks::GL_AREA_REGISTRY.lock() {
                areas.push(callbacks::GtkGLAreaPtr(
                    area.as_ptr() as *mut gtk4::ffi::GtkGLArea
                ));
            }
            // Register GLArea → surface mapping for notify::position focus restore
            if let Ok(mut gl_to_surface) = callbacks::GL_TO_SURFACE.lock() {
                gl_to_surface.insert(area.as_ptr() as usize, surface as usize);
            }

            // Request first render.
            area.queue_render();
        }
    });

    // ── GtkGLArea::unrealize — free renderer GL resources before context dies ──
    // Only touches the surface while the cell still holds it. teardown_pane clears the
    // cell before freeing the surface and before the widget leaves the tree.
    {
        let cell_unrealize = surface_cell.clone();
        gl_area.connect_unrealize(move |area| {
            // Make GL context current so Ghostty can properly free GL objects.
            area.make_current();
            if let Some(surface) = *cell_unrealize.borrow() {
                unsafe {
                    ffi::ghostty_surface_display_unrealized(surface);
                }
            }
        });
    }

    // ── GtkGLArea::render ────────────────────────────────────────────────────
    // Called by GTK frame clock when queue_render() was requested.
    gl_area.connect_render({
        let cell = surface_cell.clone();
        move |_area, _ctx| {
            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_draw(surface);
                }
            }
            gtk4::glib::Propagation::Stop // suppress GTK default render
        }
    });

    // ── GtkGLArea::resize ────────────────────────────────────────────────────
    // GTK provides logical (CSS) pixels; Ghostty needs physical pixels (Pitfall 5).
    //
    // CRITICAL: ghostty_surface_set_size must be called SYNCHRONOUSLY in this
    // signal handler, not deferred to an idle. Ghostty's renderer anti-flicker
    // guard in drawFrame() compares GL_VIEWPORT (the actual widget size) against
    // the renderer's cached screen size. If we defer set_size to an idle, the
    // next drawFrame(true) — triggered by queue_render — sees a size mismatch
    // and re-presents the last frame forever (the guard returns before updating
    // the renderer's cached size). This matches Ghostty's own GTK apprt which
    // calls sizeCallback directly in glareaResize, not deferred.
    //
    // sizeCallback early-returns when the size hasn't changed, so redundant
    // calls during rapid drag are cheap (just a comparison, no reflow).
    //
    // Do NOT bounce focus (false→true) here. The cursor blink timer is
    // independent of resize — ghostty_surface_set_size only calls setScreenSize
    // on the renderer; it does not cancel the blink timer. A false→true bounce
    // kills the timer via an async cancel race (see restore_active_pane_focus).
    {
        let cell = surface_cell.clone();
        gl_area.connect_resize(move |area, logical_w, logical_h| {
            let scale = area.scale_factor();
            let phys_w = (logical_w * scale) as u32;
            let phys_h = (logical_h * scale) as u32;

            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_set_size(surface, phys_w, phys_h);
                }
            }

            // Drive the render loop directly — wakeup idles can be starved during
            // rapid resize events (sustained mouse drag floods the GLib main loop
            // with motion events at DEFAULT priority, delaying DEFAULT_IDLE wakeup
            // idles). Calling app_tick + queue_render here ensures the terminal
            // reflows and re-renders even during a sustained resize drag.
            let app_ptr =
                crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
            if app_ptr != 0 {
                unsafe {
                    let app = app_ptr as ffi::ghostty_app_t;
                    ffi::ghostty_app_tick(app);
                }
            }
            if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                for area_ptr in areas.iter() {
                    let area: glib::translate::Borrowed<gtk4::GLArea> =
                        unsafe { glib::translate::from_glib_borrow(area_ptr.0) };
                    if area.is_realized() {
                        area.queue_render();
                        area.queue_draw(); // Gap 1B: repaints CSS border
                    }
                }
            }
        });
    }

    // ── notify::scale-factor (GHOST-06) ─────────────────────────────────────
    // Fires when the window moves to a monitor with a different DPI.
    // Must use connect_notify_local: ghostty_surface_t is *mut c_void (not Send+Sync).
    // connect_notify_local only requires 'static, and runs on the GLib main thread.
    gl_area.connect_notify_local(Some("scale-factor"), {
        let cell = surface_cell.clone();
        move |widget, _| {
            if let Some(surface) = *cell.borrow() {
                // Prefer fractional scale from GdkSurface (Wayland fractional scaling)
                // over Widget::scale_factor() which returns the integer ceiling.
                let scale = widget.native()
                    .and_then(|n| n.surface())
                    .map(|s| s.scale())
                    .unwrap_or(widget.scale_factor() as f64);
                unsafe {
                    ffi::ghostty_surface_set_content_scale(surface, scale, scale);
                    ffi::ghostty_surface_refresh(surface); // trigger redraw at new scale
                }
            }
        }
    });

    // ── Key input (GHOST-03) ─────────────────────────────────────────────────────
    // EventControllerKey fires key-pressed and key-released events.
    // CRITICAL: no allocations in this path — per CLAUDE.md typing-latency-sensitive paths.
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.connect_key_pressed({
        let cell = surface_cell.clone();
        move |_ctrl, keyval, keycode, state| {
            use crate::ghostty::input::map_mods;

            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return gtk4::glib::Propagation::Proceed,
            };

            // text field: UTF-8 from the keyval (what the key produces with modifiers applied).
            // Must be a C string. Use a stack-allocated buffer to avoid heap allocation.
            let unicode = keyval.to_unicode();
            let mut text_buf = [0u8; 8]; // UTF-8: max 4 bytes + null
            let text_ptr = if let Some(ch) = unicode {
                let mut s = [0u8; 5];
                let encoded = ch.encode_utf8(&mut s[..4]);
                let len = encoded.len();
                text_buf[..len].copy_from_slice(encoded.as_bytes());
                text_buf[len] = 0;
                text_buf.as_ptr() as *const i8
            } else {
                std::ptr::null()
            };

            let mut input = unsafe { std::mem::zeroed::<ffi::ghostty_input_key_s>() };
            // keycode must be the raw GTK hardware keycode (XKB scancode).
            // Ghostty looks this up in its own native keycodes table to resolve the physical key.
            // Do NOT translate to ghostty_input_key_e here — that is an entirely different type.
            input.keycode = keycode;
            input.mods = map_mods(state);
            input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_PRESS;
            input.text = text_ptr;
            input.consumed_mods = 0; // Not used in Phase 1

            unsafe {
                ffi::ghostty_surface_key(surface, input);
            }
            gtk4::glib::Propagation::Stop // Inhibit: prevent GTK from handling the key
        }
    });
    key_controller.connect_key_released({
        let cell = surface_cell.clone();
        move |_ctrl, _keyval, keycode, state| {
            use crate::ghostty::input::map_mods;

            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };

            let mut input = unsafe { std::mem::zeroed::<ffi::ghostty_input_key_s>() };
            input.keycode = keycode;
            input.mods = map_mods(state);
            input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_RELEASE;
            input.text = std::ptr::null();
            input.consumed_mods = 0; // Not used in Phase 1
            unsafe {
                ffi::ghostty_surface_key(surface, input);
            }
        }
    });
    gl_area.add_controller(key_controller);

    // ── Mouse button input (GHOST-04) ────────────────────────────────────────────
    let click_gesture = gtk4::GestureClick::new();
    click_gesture.set_button(0); // 0 = listen to all mouse buttons
    click_gesture.connect_pressed({
        let cell = surface_cell.clone();
        move |gesture, _n_press, _x, _y| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };
            let button = match gesture.current_button() {
                1 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_LEFT,
                2 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_MIDDLE,
                3 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_RIGHT,
                _ => return,
            };
            let mods = crate::ghostty::input::map_mods(gesture.current_event_state());
            unsafe {
                ffi::ghostty_surface_mouse_button(
                    surface,
                    ffi::ghostty_input_mouse_state_e_GHOSTTY_MOUSE_PRESS,
                    button,
                    mods,
                );
            }
        }
    });
    click_gesture.connect_released({
        let cell = surface_cell.clone();
        move |gesture, _n_press, _x, _y| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };
            let button = match gesture.current_button() {
                1 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_LEFT,
                2 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_MIDDLE,
                3 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_RIGHT,
                _ => return,
            };
            let mods = crate::ghostty::input::map_mods(gesture.current_event_state());
            unsafe {
                ffi::ghostty_surface_mouse_button(
                    surface,
                    ffi::ghostty_input_mouse_state_e_GHOSTTY_MOUSE_RELEASE,
                    button,
                    mods,
                );
            }
        }
    });
    gl_area.add_controller(click_gesture);

    // ── Mouse motion ─────────────────────────────────────────────────────────────
    let motion_controller = gtk4::EventControllerMotion::new();
    motion_controller.connect_motion({
        let cell = surface_cell.clone();
        move |ctrl, x, y| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };
            let mods = crate::ghostty::input::map_mods(ctrl.current_event_state());
            unsafe {
                crate::ghostty::ffi::ghostty_surface_mouse_pos(surface, x, y, mods);
            }
        }
    });
    gl_area.add_controller(motion_controller);

    // ── Focus tracking (GHOST-05) ─────────────────────────────────────────────
    // EventControllerFocus fires enter/leave when GTK keyboard focus enters/leaves the widget.
    // This ensures ghostty_surface_set_focus() stays in sync with GTK focus routing —
    // critical after GtkPaned drags (separator steals focus) and sidebar show/hide.
    // Without this, Ghostty's internal focused flag diverges from GTK reality, and
    // subsequent set_focus(true) calls hit the early-return guard (if self.focused == focused { return; }).
    let focus_controller = gtk4::EventControllerFocus::new();
    focus_controller.connect_enter({
        let cell = surface_cell.clone();
        // Weak: a strong clone here would make GLArea → controller → closure → GLArea a cycle,
        // so a closed pane would never be finalized.
        let gl_area_for_focus = gl_area.downgrade();
        move |_ctrl| {
            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, true);
                    // Kick the render loop so the cursor becomes visible immediately
                    // rather than waiting up to one blink interval (~500ms). The
                    // renderer thread processes the focused=true message asynchronously;
                    // without a refresh+queue_render here, GTK renders that happen
                    // before the message is processed show the stale (invisible) cursor.
                    ffi::ghostty_surface_refresh(surface);
                }
                if let Some(area) = gl_area_for_focus.upgrade() {
                    area.queue_render();
                }
            }
        }
    });
    focus_controller.connect_leave({
        let cell = surface_cell.clone();
        move |_ctrl| {
            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, false);
                }
            }
        }
    });
    gl_area.add_controller(focus_controller);

    // ── Scroll input ─────────────────────────────────────────────────────────────
    let scroll_controller = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::DISCRETE,
    );
    scroll_controller.connect_scroll({
        let cell = surface_cell.clone();
        move |ctrl, dx, dy| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return gtk4::glib::Propagation::Proceed,
            };
            // Detect if this is pixel-precise (touchpad) or discrete (mouse wheel)
            let is_pixel = ctrl
                .current_event()
                .and_then(|e| e.downcast::<gtk4::gdk::ScrollEvent>().ok())
                .map(|se| se.direction() == gtk4::gdk::ScrollDirection::Smooth)
                .unwrap_or(false);

            // ghostty_input_scroll_mods_t is a bitmask:
            // bit 0: scroll_is_pixel (1 if touchpad, 0 if mouse wheel)
            // bit 1: momentum (1 if momentum scrolling)
            let scroll_mods = if is_pixel { 1 } else { 0 };

            unsafe {
                ffi::ghostty_surface_mouse_scroll(surface, dx, dy, scroll_mods);
            }
            gtk4::glib::Propagation::Stop
        }
    });
    gl_area.add_controller(scroll_controller);

    PaneSurface {
        gl_area,
        surface: surface_cell,
        ctx,
    }
}

// ── Clipboard callbacks ──────────────────────────────────────────────────────
//
// Ghostty starts every clipboard read through read_clipboard_cb with confirmed=false.
// A safe paste then completes. An unsafe paste, or an OSC 52 read from terminal output,
// comes back through confirm_read_clipboard_cb, which tells them apart by request type.
// Each callback takes the pane from its own userdata, so a request is always completed
// on the surface that made it.

pub(crate) unsafe extern "C" fn read_clipboard_cb(
    userdata: *mut std::ffi::c_void,
    clipboard_type: crate::ghostty::ffi::ghostty_clipboard_e,
    request: *mut std::ffi::c_void,
) {
    use gtk4::prelude::*;

    let Some(ctx) = (unsafe { pane_ctx(userdata) }) else {
        return;
    };
    let Some(surface) = *ctx.surface.borrow() else {
        return;
    };

    // Read clipboard text synchronously using GLib event loop.
    // gtk4::glib::MainContext::block_on runs the async future on the current (main) thread.
    // This is safe here because read_clipboard_cb is called from the GLib main thread.
    let text: Option<glib::GString> = gtk4::gdk::Display::default().and_then(|display| {
        let clipboard = if clipboard_type == ffi::ghostty_clipboard_e_GHOSTTY_CLIPBOARD_SELECTION {
            display.primary_clipboard()
        } else {
            display.clipboard()
        };
        glib::MainContext::default()
            .block_on(clipboard.read_text_future())
            .ok()
            .flatten()
    });

    let c_text = text.and_then(|s| std::ffi::CString::new(s.as_str()).ok());
    let text_ptr = match c_text.as_ref() {
        Some(s) => s.as_ptr(),
        // Always answer: a null string would crash Ghostty, and an unanswered request leaks.
        None => EMPTY_C_STR.as_ptr() as *const std::ffi::c_char,
    };

    // confirmed=false: ghostty asks for confirmation (via confirm_read_clipboard_cb)
    // instead of completing OSC 52 reads or unsafe pastes on its own.
    unsafe {
        ffi::ghostty_surface_complete_clipboard_request(surface, text_ptr, request, false);
    }
}

pub(crate) unsafe extern "C" fn confirm_read_clipboard_cb(
    userdata: *mut std::ffi::c_void,
    value: *const std::os::raw::c_char,
    request: *mut std::ffi::c_void,
    request_type: crate::ghostty::ffi::ghostty_clipboard_request_e,
) {
    let Some(ctx) = (unsafe { pane_ctx(userdata) }) else {
        return;
    };
    let Some(surface) = *ctx.surface.borrow() else {
        return;
    };

    let data = match request_type {
        // The user pressed paste and the text needs confirming (multi-line or control
        // characters). Paste only starts from the user's own paste action, so allow it.
        ffi::ghostty_clipboard_request_e_GHOSTTY_CLIPBOARD_REQUEST_PASTE => value,
        // Terminal output asked to read the clipboard (OSC 52). Deny it: an empty reply goes
        // back to the program and the clipboard contents never leave the process. Completing
        // with confirmed=false here would make ghostty ask again, so the deny must be
        // confirmed=true with empty data.
        ffi::ghostty_clipboard_request_e_GHOSTTY_CLIPBOARD_REQUEST_OSC_52_READ => {
            EMPTY_C_STR.as_ptr() as *const std::os::raw::c_char
        }
        // Other request types never reach this path. Leave them unanswered rather than
        // complete them with data they did not ask for.
        _ => return,
    };
    unsafe {
        ffi::ghostty_surface_complete_clipboard_request(surface, data, request, true);
    }
}

pub(crate) unsafe extern "C" fn write_clipboard_cb(
    userdata: *mut std::ffi::c_void,
    clipboard_type: crate::ghostty::ffi::ghostty_clipboard_e,
    content: *const crate::ghostty::ffi::ghostty_clipboard_content_s,
    _len: usize,
    _confirm: bool,
) {
    use gtk4::prelude::*;

    // Only the focused pane may write the clipboard. OSC 52 writes are program output, so a
    // background pane (or a process running in it) must not overwrite the clipboard the user
    // is working with. Copy from the focused pane is unaffected. This focus gate is also why
    // the `confirm` flag is ignored.
    let focused = unsafe { pane_ctx(userdata) }
        .and_then(|ctx| ctx.gl_area.upgrade())
        .map(|area| area.has_css_class("active-pane"))
        .unwrap_or(false);
    if !focused {
        return;
    }

    if content.is_null() {
        return;
    }
    let item = &*content;
    let text = if !item.data.is_null() {
        match std::ffi::CStr::from_ptr(item.data).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    } else {
        return;
    };

    let display = match gtk4::gdk::Display::default() {
        Some(d) => d,
        None => return,
    };
    let clipboard = if clipboard_type == ffi::ghostty_clipboard_e_GHOSTTY_CLIPBOARD_SELECTION {
        display.primary_clipboard()
    } else {
        display.clipboard()
    };
    clipboard.set_text(&text);
}
