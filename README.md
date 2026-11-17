# lowbattery

Battery monitor for wireless peripherals on Windows 10/11 and Linux, written in Rust with a native [CreamUI](https://github.com/sammwyy/creamui) UI. It runs as a single process, installs no services, sends no telemetry, and needs neither administrator rights nor the vendor's software.

- **Standard Bluetooth:** reads the same level that Windows Settings shows.
- **Logitech HID++ 2.0:** Unifying, Bolt and Lightspeed receivers, and direct connection.
- **SteelSeries:** Aerox, Prime, Rival 3 Wireless, Arctis 7/7P/7+/Nova 5/Nova 7/Nova Pro Wireless.
- **Razer:** wireless mice and keyboards and their dongles, with no model table.
- **Keyboards with the `FF1C:0092` battery channel, any brand:** Telink/Evision firmware sold by FEKER, VGN, Rainy and others. They are recognized by that channel, not by brand, over both 2.4G and cable, with level and charging state. There is a single card per keyboard, whether it uses the cable or the receiver.
- **Generic 2.4G receivers without battery** (for example YICHIP): they show as connected, with a note that they do not report battery, and they are not shown in the widget.
- **Any brand:** HID devices that declare the standard "Battery Strength" usage.
- **Tray:** the icon shows the lowest level and the tooltip lists all devices. A Windows toast warns when a device reaches low battery.
- **Floating widget:** borderless, with configurable opacity, an always-on-top option and a choice of which devices to show.

## Build (Ubuntu, Docker only)

```sh
./build.sh test        # unit tests (protocols with simulated devices)
./build.sh check       # clippy for Windows
./build.sh release     # dist/lowbattery.exe
./build.sh installer   # dist/lowbattery-0.1.0.0.msix (Store) and -sideload.msix (signed for testing)
```

UI smoke test with Wine (`--demo` uses simulated devices):

```sh
docker build -t lowbattery-wine -f docker/Dockerfile.wine docker/
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/src lowbattery-wine /src/docker/smoke.sh
# screenshots in dist/smoke-*.png
```

## MSIX and Microsoft Store

`./build.sh installer` builds the package with `makemsix`, Microsoft's official tool, which also works on Linux. The first run builds the `docker/Dockerfile.msix` image. It produces three files:

- **`dist/lowbattery-<ver>.msix`:** unsigned; this is the one you upload to Partner Center, because the Store signs it.
- **`dist/lowbattery-<ver>-sideload.msix`:** signed with a self-made development certificate, to install on your PC.
- **`dist/lowbattery-dev.cer`:** that certificate.

**For the Store:**
1. Reserve the name in Partner Center.
2. Copy `installer/msix.env.example` to `installer/msix.env` and paste `Name`, `Publisher` and `PublisherDisplayName` from *Product identity*.
3. Run `./build.sh installer` again and upload the unsigned `.msix`.

The app declares `runFullTrust`, which is required for HID access; Microsoft reviews it during certification.

**To install it on your PC (sideload):**
1. Trust the certificate once, from PowerShell as administrator:
   `Import-Certificate -FilePath lowbattery-dev.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople`
2. Double-click the `-sideload.msix`, or install it with `Add-AppxPackage`.

Differences when the app runs packaged:
- **Config and log:** stored in `%LOCALAPPDATA%\Packages\<package>\LocalState\`.
- **Start with Windows:** uses the package's `StartupTask` instead of the `Run` key, and can be disabled from Task Manager > Startup.
- **Notifications:** sent with the package identity.

## Usage

- `lowbattery.exe`: opens the main window, the widget and the tray icon. Closing the window hides it to the tray.
- `--minimized`: starts with the tray only. Start with Windows uses this mode.
- `--demo`: uses simulated devices, no hardware.
- `--debug`: verbose logging.
- Configuration: `%APPDATA%\LowBattery\config.json` (packaged: see above).

## Linux

```sh
./build.sh linux   # dist/lowbattery_<ver>_amd64.deb and dist/lowbattery-<ver>-linux-amd64.tar.gz
```

- **`.deb` (Ubuntu, Debian, Mint, Pop!_OS):** `sudo apt install ./dist/lowbattery_*_amd64.deb`. Also installs the udev rule.
- **`.tar.gz` (any distro):** extract and run `sudo ./install.sh`, which installs into `/usr/local`. To uninstall: `sudo ./install.sh --uninstall`.
- **Requirements:** glibc 2.35 or later (Ubuntu 22.04+, Debian 12+, Fedora 36+) and X11 or Wayland.

What changes compared to Windows:
- **Logitech, Bluetooth and standard HID:** read from `/sys/class/power_supply`, which the kernel publishes (drivers `hid-logitech-hidpp` and `hid-input`). No permissions needed.
- **SteelSeries, Razer and `FF1C` keyboards:** read through `/dev/hidraw*`, which is root-only until `installer/linux/70-lowbattery.rules` is installed. The packages install it. If it is missing, the log shows `no permission to open /dev/hidrawN`.
- **Tray:** uses StatusNotifierItem. Works on KDE, XFCE, Cinnamon and Ubuntu's GNOME. Stock GNOME needs the *AppIndicator* extension; without a tray, the main window stays visible.
- **Wayland:** the protocol does not let an app choose its window position or keep it always on top, so those widget options only work on X11.
- **Config and log:** `~/.config/LowBattery/`.

### Snap Store and Flathub

- **Snap:** `snap/snapcraft.yaml`. Build it with `snapcraft` and publish it with `snapcraft register lowbattery` and `snapcraft upload --release=stable`. Strict confinement gives no hidraw access, so the snap only supports Logitech, Bluetooth and standard HID.
- **Flathub:** `installer/flatpak/io.github.gonzxaj.LowBattery.yml`. First run `installer/flatpak/gen-sources.sh` (generates `cargo-sources.json`), then `flatpak-builder --user --install build-flatpak installer/flatpak/io.github.gonzxaj.LowBattery.yml`. To publish, open a PR on `flathub/flathub` with the manifest (with a `git` source and tag) and `cargo-sources.json`. SteelSeries and Razer still need the udev rule installed on the system.
- **Shared metadata:** `installer/linux/` holds the `.desktop` file, the AppStream metainfo (Flathub requires at least one screenshot: `assets/screenshots/devices.png`), the icon and the udev rule.

## Log and missing peripherals

The app writes `%APPDATA%\LowBattery\lowbattery.log`, which you can open from the tray or with the "View log" button. It contains:

- **At startup:** a diagnostic with the HID interfaces of supported vendors and the Bluetooth nodes with battery.
- **State changes:** each new device and each change in level, charging or connection.
- **Unsupported peripherals:** an `UNSUPPORTED peripheral vid_xxxx&pid_xxxx "Product" ...` line for each detected peripheral that has no battery protocol yet, with what is needed to add it. If a device is missing, send that file.

With `--debug` it also logs every read cycle.

Architecture, APIs, compatibility, limitations and the real-hardware test plan are in [docs/RESEARCH.md](docs/RESEARCH.md).

## License

Copyright (c) 2026 Gonzalo Carranza (Gonzxaj). Released under the [MIT license](LICENSE).

Includes a modified copy of `creamui-render` (Apache-2.0) and other open-source dependencies; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
