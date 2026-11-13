# third_party

Both copies come from creamui `main` at commit `cbc63b9` (2026-11-13; newer than the
0.3.0 release on crates.io but still numbered 0.3.0), packaged with `cargo package`
so they build outside creamui's workspace. The root `Cargo.toml` pulls the rest of
creamui from that same commit and swaps these two in for both crates.io and git.

To move to a newer commit: package `creamui-render` and `creamui-platform` from it,
re-apply our diff (what differs from the packaged sources), and bump the `rev`.
Only what the build reads is kept: no `examples/`, `tests/`, `Cargo.lock` or
`Cargo.toml.orig` (and their `[[example]]`/`[[test]]` entries are removed).

The restore repaint (Windows, X11 map), Wayland programmatic resize and Linux
`app_id` fixes we used to carry are upstream since `cbc63b9` (PRs #4, #5, #6).
What is left is below. Remove both directories and their `[patch]` entries in the
root Cargo.toml once creamui ships these too (or LowBattery stops needing them).

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

