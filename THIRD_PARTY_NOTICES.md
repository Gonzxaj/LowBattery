# Third-party notices

LowBattery is MIT-licensed (see `LICENSE`). It includes or builds on the
following third-party work.

## CreamUI (Apache-2.0), modified copy

`third_party/creamui-render/` is a copy of **creamui-render 0.3.0**, part of
[CreamUI](https://github.com/sammwyy/creamui) by Sammwy, licensed under the
Apache License 2.0 (full text in `third_party/creamui-render/LICENSE`).

**Changes:** `src/window.rs` adds `Pipeline::invalidate_surface` and calls it when
a window is restored from minimized and from `WindowHandle::show`, so Windows
repaints the full window instead of leaving it blank or black, and
`WindowHandle::resize` adopts sizes Wayland applies immediately (no `Resized`
event), so a resized window isn't drawn stretched. The modified file
carries a notice at its top. The rest of CreamUI is used unmodified from crates.io.

## CreamUI platform (Apache-2.0), modified copy

`third_party/creamui-platform/` is a copy of **creamui-platform 0.3.0** from the same
project and license (full text in `third_party/creamui-platform/LICENSE`).

**Changes:** `src/winit.rs` adds `set_app_id`, which sets the Wayland `app_id` and
X11 `WM_CLASS` of new windows so Linux docks match them to the app's `.desktop`
file. The modified file carries a notice at its top.

## Device protocol references

- **rivalcfg** (<https://github.com/flozz/rivalcfg>), WTFPL: battery commands
  for SteelSeries mice.
- **HeadsetControl** (GPL-3.0), **Solaar** and the Linux kernel `hid-logitech-hidpp`
  driver (GPL-2.0), **OpenRazer** (GPL-2.0): consulted only as documentation of
  device protocols (report layouts, command bytes). No code from them is included.
- **MyFinder Device Battery** (<https://github.com/wushuai0802/myfinder-device-battery>),
  AGPL-3.0: its protocol notes for the Telink/VGN `320F:5088` battery query were
  consulted as documentation and verified on a real receiver. No code is included.

## Rust crates

Compiled into the Windows build (generated with `cargo metadata` for
`x86_64-pc-windows-gnu`). Each crate's full license text ships in its source
package on crates.io.

<!-- shortcut: SPDX ids only; switch to cargo-about to bundle every license text if a distributor requires it. -->

| Crate | Version | License |
|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| arboard | 3.7.0 | MIT OR Apache-2.0 |
| arrayref | 0.3.9 | BSD-2-Clause |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| clipboard-win | 5.4.3 | BSL-1.0 |
| crc32fast | 1.5.4 | MIT OR Apache-2.0 |
| creamui | 0.3.0 | Apache-2.0 |
| creamui-core | 0.3.0 | Apache-2.0 |
| creamui-fonts | 0.3.0 | Apache-2.0 |
| creamui-image | 0.3.0 | Apache-2.0 |
| creamui-platform | 0.3.0 | Apache-2.0 |
| creamui-reactive | 0.3.0 | Apache-2.0 |
| creamui-render | 0.3.0 | Apache-2.0 |
| creamui-theme | 0.3.0 | Apache-2.0 |
| creamui-widgets | 0.3.0 | Apache-2.0 |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| error-code | 3.4.0 | BSL-1.0 |
| fax | 0.2.7 | MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| flate2 | 1.1.11 | MIT OR Apache-2.0 |
| font-types | 0.12.7 | MIT OR Apache-2.0 |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 |
| grid | 0.15.0 | MIT |
| half | 2.7.1 | MIT OR Apache-2.0 |
| hidapi | 2.6.7 | MIT |
| image | 0.25.10 | MIT OR Apache-2.0 |
| itoa | 1.0.19 | MIT OR Apache-2.0 |
| keyboard-types | 0.8.3 | MIT OR Apache-2.0 |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| log | 0.4.35 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| memmap2 | 0.9.12 | MIT OR Apache-2.0 |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| muda | 0.21.2 | Apache-2.0 OR MIT |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| png | 0.17.16 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| quick-error | 2.0.1 | MIT/Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| read-fonts | 0.41.0 | MIT OR Apache-2.0 |
| rfd | 0.17.2 | MIT |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 |
| simd-adler32 | 0.3.10 | MIT |
| skrifa | 0.44.0 | MIT OR Apache-2.0 |
| slotmap | 1.1.1 | Zlib |
| smol_str | 0.2.2 | MIT OR Apache-2.0 |
| softbuffer | 0.4.8 | MIT OR Apache-2.0 |
| strict-num | 0.1.1 | MIT |
| swash | 0.2.10 | Apache-2.0 OR MIT |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.10 | MIT OR Apache-2.0 |
| taffy | 0.7.7 | MIT |
| tauri-winrt-notification | 0.8.1 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| tiff | 0.11.3 | MIT |
| tiny-skia | 0.11.4 | BSD-3-Clause |
| tiny-skia-path | 0.11.4 | BSD-3-Clause |
| tracing | 0.1.44 | MIT |
| tracing-core | 0.1.36 | MIT |
| tray-icon | 0.26.1 | MIT OR Apache-2.0 |
| unicode-bidi | 0.3.18 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| weezl | 0.1.12 | MIT OR Apache-2.0 |
| windows | 0.62.2 | MIT OR Apache-2.0 |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 |
| windows-core | 0.62.2 | MIT OR Apache-2.0 |
| windows-future | 0.3.2 | MIT OR Apache-2.0 |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 |
| windows-link | 0.2.1 | MIT OR Apache-2.0 |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 |
| windows-result | 0.4.1 | MIT OR Apache-2.0 |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 |
| windows-version | 0.1.7 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 |
| winit | 0.30.13 | Apache-2.0 |
| yazi | 0.2.2 | Apache-2.0 OR MIT |
| zeno | 0.3.3 | Apache-2.0 OR MIT |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zlib-rs | 0.6.8 | Zlib |
| zmij | 1.0.23 | MIT |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |

### Linux-only crates

Also compiled into the Linux build (`cargo tree` for `x86_64-unknown-linux-gnu`,
minus the crates listed above).

| Crate | Version | License |
|---|---|---|
| ab_glyph | 0.2.32 | Apache-2.0 |
| ab_glyph_rasterizer | 0.1.10 | Apache-2.0 |
| ahash | 0.8.12 | MIT OR Apache-2.0 |
| arc-swap | 1.9.5 | MIT OR Apache-2.0 |
| as-raw-xcb-connection | 1.0.1 | MIT OR Apache-2.0 |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 |
| async-channel | 2.5.0 | Apache-2.0 OR MIT |
| async-executor | 1.14.1 | Apache-2.0 OR MIT |
| async-io | 2.6.0 | Apache-2.0 OR MIT |
| async-lock | 3.5.0 | Apache-2.0 OR MIT |
| async-process | 2.5.0 | Apache-2.0 OR MIT |
| async-recursion | 1.2.0 | MIT OR Apache-2.0 |
| async-signal | 0.2.14 | Apache-2.0 OR MIT |
| async-task | 4.7.1 | Apache-2.0 OR MIT |
| async-trait | 0.1.92 | MIT OR Apache-2.0 |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| basic-udev | 0.1.3 | MIT OR Apache-2.0 |
| blocking | 1.7.0 | Apache-2.0 OR MIT |
| calloop | 0.13.0 | MIT |
| calloop-wayland-source | 0.3.0 | MIT |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT |
| crossbeam-queue | 0.3.14 | MIT OR Apache-2.0 |
| ctor | 0.10.1 | Apache-2.0 OR MIT |
| dlib | 0.5.3 | MIT |
| downcast-rs | 1.2.1 | MIT/Apache-2.0 |
| drm | 0.14.1 | MIT |
| drm-ffi | 0.9.1 | MIT |
| drm-fourcc | 2.2.0 | MIT |
| drm-sys | 0.8.1 | MIT |
| endi | 1.1.1 | MIT |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| event-listener | 5.4.2 | Apache-2.0 OR MIT |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT |
| fastrand | 2.5.0 | Apache-2.0 OR MIT |
| futures-io | 0.3.34 | MIT OR Apache-2.0 |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 |
| gethostname | 1.1.0 | Apache-2.0 |
| getrandom | 0.3.4 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hex | 0.4.3 | MIT OR Apache-2.0 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| ksni | 0.3.7 | Unlicense |
| libloading | 0.8.9 | ISC |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| linux-raw-sys | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| linux-raw-sys | 0.9.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| nix | 0.30.1 | MIT |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 |
| owned_ttf_parser | 0.25.1 | Apache-2.0 |
| parking | 2.2.1 | Apache-2.0 OR MIT |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| pastey | 0.2.3 | MIT OR Apache-2.0 |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| piper | 0.2.5 | MIT OR Apache-2.0 |
| polling | 3.11.0 | Apache-2.0 OR MIT |
| pollster | 0.4.0 | Apache-2.0/MIT |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 |
| quick-xml | 0.41.0 | MIT |
| rustix | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustversion | 1.0.23 | MIT OR Apache-2.0 |
| scoped-tls | 1.0.1 | MIT/Apache-2.0 |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| sctk-adwaita | 0.10.1 | MIT |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.16.5 | MIT OR Apache-2.0 |
| smithay-client-toolkit | 0.19.2 | MIT |
| task-local | 0.1.1 | MIT OR Apache-2.0 |
| thiserror | 1.0.69 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 |
| tiny-xlib | 0.2.5 | MIT OR Apache-2.0 OR Zlib |
| toml_datetime | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_edit | 0.25.19+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| tracing-attributes | 0.1.31 | MIT |
| ttf-parser | 0.25.1 | MIT OR Apache-2.0 |
| uuid | 1.29.2 | Apache-2.0 OR MIT |
| wayland-backend | 0.3.17 | MIT |
| wayland-client | 0.31.15 | MIT |
| wayland-csd-frame | 0.3.0 | MIT |
| wayland-cursor | 0.31.14 | MIT |
| wayland-protocols | 0.32.13 | MIT |
| wayland-protocols-plasma | 0.3.12 | MIT |
| wayland-protocols-wlr | 0.3.12 | MIT |
| wayland-scanner | 0.31.11 | MIT |
| wayland-sys | 0.31.11 | MIT |
| winnow | 1.0.5 | MIT |
| x11-dl | 2.21.0 | MIT |
| x11rb | 0.13.2 | MIT OR Apache-2.0 |
| x11rb-protocol | 0.13.2 | MIT OR Apache-2.0 |
| xcursor | 0.3.11 | MIT |
| xkbcommon-dl | 0.4.2 | MIT |
| xkeysym | 0.2.1 | MIT OR Apache-2.0 OR Zlib |
| zbus | 5.19.0 | MIT |
| zbus_macros | 5.19.0 | MIT |
| zbus_names | 4.3.5 | MIT |
| zcheapstr | 1.2.0 | MIT |
| zvariant | 5.15.1 | MIT |
| zvariant_derive | 5.15.1 | MIT |
| zvariant_utils | 4.2.0 | MIT |
