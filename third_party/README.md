# third_party

Both copies come from creamui `main` at commit `a36a4bc` (2026-11-10; newer than the
0.3.0 release on crates.io but still numbered 0.3.0), packaged with `cargo package`
so they build outside creamui's workspace. The root `Cargo.toml` pulls the rest of
creamui from that same commit and swaps these two in for both crates.io and git.

To move to a newer commit: package `creamui-render` and `creamui-platform` from it,
re-apply our diff (what differs from the packaged sources), and bump the `rev`.
Only what the build reads is kept: no `examples/`, `tests/`, `Cargo.lock` or
`Cargo.toml.orig` (and their `[[example]]`/`[[test]]` entries are removed).

`creamui-render/` is creamui-render with two fixes.

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

`creamui-platform/` is creamui-platform with one addition: `set_app_id()` (in `src/winit.rs`, re-exported from `lib.rs`), applied as
the Wayland `app_id` / X11 `WM_CLASS` of every window. Without it GNOME/KDE docks
show the app as "Unknown" with a generic icon, because they match windows to their
`.desktop` file by that id. LowBattery calls it on Linux before the first window.

Upstream fix for the restore repaint: branch `bug-minimize-win32` of github.com/sammwyy/creamui.
Remove each directory and its `[patch.crates-io]` entry in the root Cargo.toml
once a creamui release includes the fix.
