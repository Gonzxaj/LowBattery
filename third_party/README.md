# third_party

`creamui-render/` is creamui-render 0.3.0 from crates.io (Apache-2.0) with one fix:
on Windows, a window restored from minimized came back black, and one shown again
after `hide()` (closed to the tray, reopened) came back blank, because the renderer
only presents what changed since the last frame and Windows had discarded the
pixels. See `Pipeline::invalidate_surface` in `src/window.rs`.

Upstream fix: branch `bug-minimize-win32` of github.com/sammwyy/creamui.
Remove this directory and the `[patch.crates-io]` entry in the root Cargo.toml
once a creamui release includes it.
