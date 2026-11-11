#!/bin/sh
# Installs LowBattery from this tarball:  sudo ./install.sh
# Uninstall: sudo ./install.sh --uninstall
# PREFIX defaults to /usr/local; the udev rule always goes to /etc/udev/rules.d.
set -e
cd "$(dirname "$0")"
PREFIX=${PREFIX:-/usr/local}
ID=io.github.gonzxaj.LowBattery
RULE=/etc/udev/rules.d/70-lowbattery.rules
FILES="bin/lowbattery share/applications/$ID.desktop share/metainfo/$ID.metainfo.xml share/icons/hicolor/256x256/apps/$ID.png"

if [ "$1" = "--uninstall" ]; then
  for f in $FILES; do rm -f "$PREFIX/$f"; done
  rm -f "$RULE"
else
  for f in $FILES; do install -Dm644 "usr/$f" "$PREFIX/$f"; done
  chmod 755 "$PREFIX/bin/lowbattery"
  install -Dm644 usr/lib/udev/rules.d/70-lowbattery.rules "$RULE"
fi
if command -v udevadm >/dev/null; then
  udevadm control --reload-rules || true
  udevadm trigger --subsystem-match=hidraw || true
fi
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" || true
echo "done"
