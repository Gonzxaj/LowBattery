# third_party

Both copies come from creamui `main` at commit `a36a4bc` (2026-11-10; newer than the
0.3.0 release on crates.io but still numbered 0.3.0), packaged with `cargo package`
so they build outside creamui's workspace. The root `Cargo.toml` pulls the rest of
creamui from that same commit and swaps these two in for both crates.io and git.

To move to a newer commit: package `creamui-render` and `creamui-platform` from it,
re-apply our diff (what differs from the packaged sources), and bump the `rev`.
Only what the build reads is kept: no `examples/`, `tests/`, `Cargo.lock` or
`Cargo.toml.orig` (and their `[[example]]`/`[[test]]` entries are removed).

`creamui-render/` is creamui-render with two fixes and one addition.

**Restore repaint (Windows):**
on Windows, a window restored from minimized came back black, and one shown again
after `hide()` (closed to the tray, reopened) came back blank, because the renderer
only presents what changed since the last frame and Windows had discarded the
pixels. See `Pipeline::invalidate_surface` in `src/window.rs`. On Linux X11 the
same happened at startup and on restore: the server drops the pixels on map and
asks for a repaint (Expose), which arrives as a plain `RedrawRequested` that can't
be told apart from one the renderer queued, so only what changed was presented
(at login, with device data loading, the widget's transparent corners stayed opaque
black). `creamui-platform` now forwards winit's `Occluded` (X11 VisibilityNotify,
sent on map) as `WindowEvent::Occluded`, and `Occluded(false)` calls
`invalidate_surface`.

**Programmatic resize (Wayland):** winit applies `request_inner_size` immediately
on Wayland and emits no `Resized`, so after `WindowHandle::resize` the renderer
kept drawing at the old size and the compositor stretched it (the floating widget
grows from 1 row to N). `WindowHandle::resize` now reads the applied size back and
updates the surface itself.

**App ID (Linux):** `WindowOptions::app_id`, passed down to the platform and
inherited by popups opened from a window.

`creamui-platform/` is creamui-platform with one addition: `WindowAttributes::app_id`,
applied as the Wayland `app_id` / X11 `WM_CLASS` of the window (`src/winit.rs`; the
native Wayland backend in `src/wayland/runtime.rs` uses it over the executable name).
Without it GNOME/KDE docks show the app as "Unknown" with a generic icon, because they
match windows to their `.desktop` file by that id. LowBattery sets it on both windows.

Upstream fixes, on github.com/sammwyy/creamui: restore repaint in branch
`bug-minimize-win32` (PR #4), programmatic resize in `fix-wayland-resize`, app ID in
`feat-app-id`.
Remove each directory and its `[patch.crates-io]` entry in the root Cargo.toml
once a creamui release includes the fix.

**Window moved event (both crates):** creamui dropped winit's `WindowEvent::Moved`.
`creamui-platform` now forwards it as `WindowEvent::Moved`, and `creamui-render` adds
`WindowHandle::on_moved(|x, y| ...)` (logical pixels, like `position()`). LowBattery
uses it to save the widget position once a drag stops, instead of sampling it on
every battery poll. Never fires on Wayland, where windows can't know their position.

**Native handles (`creamui-render`):** `WindowHandle::raw_window_handle()` and
`raw_display_handle()` expose the `raw-window-handle` values creamui already uses to
draw (Win32 HWND, Xlib window + display, Wayland surface). LowBattery uses them in
`os::decorate_window` for the window icon and to keep the floating widget out of the
taskbar / Alt+Tab: Win32 icon + `WS_EX_TOOLWINDOW` on Windows, `_NET_WM_ICON` +
`_NET_WM_STATE_SKIP_TASKBAR/SKIP_PAGER` on X11. Wayland has no equivalent.

