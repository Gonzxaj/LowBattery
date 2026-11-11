#!/usr/bin/env bash
# Runs inside lowbattery-build after `cargo build --release`. Produces:
#   dist/lowbattery_<ver>_<arch>.deb              Debian/Ubuntu; installs the udev rule
#   dist/lowbattery-<ver>-linux-<arch>.tar.gz     any distro: same tree + install.sh
# Maintainer for the .deb comes from DEB_MAINTAINER ("Name <email>").
set -euo pipefail
cd /src
ID=io.github.gonzxaj.LowBattery
VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
ARCH=$(dpkg --print-architecture)
BIN=target/release/lowbattery
: "${DEB_MAINTAINER:=Gonzxaj <gonzxaj@users.noreply.github.com>}"
# Lowest glibc the binary links against = the build image's; older distros can't run it.
GLIBC=$(objdump -T "$BIN" | grep -o 'GLIBC_[0-9.]*' | cut -d_ -f2 | sort -uV | tail -1)

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ROOT="$STAGE/lowbattery-$VER"
install -Dm755 "$BIN" "$ROOT/usr/bin/lowbattery"
install -Dm644 installer/linux/$ID.desktop -t "$ROOT/usr/share/applications"
install -Dm644 installer/linux/$ID.metainfo.xml -t "$ROOT/usr/share/metainfo"
install -Dm644 assets/emblem-tile.png "$ROOT/usr/share/icons/hicolor/256x256/apps/$ID.png"
install -Dm644 installer/linux/70-lowbattery.rules -t "$ROOT/usr/lib/udev/rules.d"
install -Dm644 LICENSE "$ROOT/usr/share/doc/lowbattery/copyright"
install -Dm644 THIRD_PARTY_NOTICES.md -t "$ROOT/usr/share/doc/lowbattery"

# --- tar.gz: the tree plus an installer for /usr/local ---
install -m755 installer/linux/install.sh "$ROOT/install.sh"
TAR="dist/lowbattery-$VER-linux-$ARCH.tar.gz"
mkdir -p dist
tar -C "$STAGE" --owner=0 --group=0 -czf "$TAR" "lowbattery-$VER"
rm "$ROOT/install.sh"

# --- .deb ---
mkdir -p "$ROOT/DEBIAN"
cat > "$ROOT/DEBIAN/control" <<CTL
Package: lowbattery
Version: $VER
Architecture: $ARCH
Maintainer: $DEB_MAINTAINER
Depends: libc6 (>= $GLIBC), libgcc-s1
Recommends: xdg-utils
Section: utils
Priority: optional
Homepage: https://github.com/Gonzxaj/LowBattery
Description: Battery levels of wireless mice, keyboards and headsets
 Shows the battery of Logitech, SteelSeries, Razer, Bluetooth and standard
 HID peripherals in a window, a floating widget and the system tray, and
 warns once when one runs low. No vendor software and no telemetry.
CTL
for s in postinst postrm; do
  printf '#!/bin/sh\nset -e\nif command -v udevadm >/dev/null; then udevadm control --reload-rules || true; udevadm trigger --subsystem-match=hidraw || true; fi\n' > "$ROOT/DEBIAN/$s"
  chmod 755 "$ROOT/DEBIAN/$s"
done
DEB="dist/lowbattery_${VER}_$ARCH.deb"
dpkg-deb --root-owner-group --build "$ROOT" "$DEB" >/dev/null
echo "glibc >= $GLIBC"
