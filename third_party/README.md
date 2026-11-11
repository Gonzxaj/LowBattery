# third_party

`creamui-render/` is creamui-render 0.3.0 from crates.io (Apache-2.0) with two fixes.

**Restore repaint (Windows):**
on Windows, a window restored from minimized came back black, and one shown again
after `hide()` (closed to the tray, reopened) came back blank, because the renderer
only presents what changed since the last frame and Windows had discarded the
pixels. See `Pipeline::invalidate_surface` in `src/window.rs`.

**Programmatic resize (Wayland):** winit applies `request_inner_size` immediately
on Wayland and emits no `Resized`, so after `WindowHandle::resize` the renderer
kept drawing at the old size and the compositor stretched it (the floating widget
grows from 1 row to N). `WindowHandle::resize` now reads the applied size back and
updates the surface itself.

`creamui-platform/` is creamui-platform 0.3.0 from crates.io (Apache-2.0) with one
addition: `set_app_id()` (in `src/winit.rs`, re-exported from `lib.rs`), applied as
the Wayland `app_id` / X11 `WM_CLASS` of every window. Without it GNOME/KDE docks
show the app as "Unknown" with a generic icon, because they match windows to their
`.desktop` file by that id. LowBattery calls it on Linux before the first window.

Upstream fix for the restore repaint: branch `bug-minimize-win32` of github.com/sammwyy/creamui.
Remove each directory and its `[patch.crates-io]` entry in the root Cargo.toml
once a creamui release includes the fix.
