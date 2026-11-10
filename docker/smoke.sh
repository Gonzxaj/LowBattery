#!/usr/bin/env bash
# Inside lowbattery-wine: start the demo build, screenshot the screen.
set -u
export HOME=$(mktemp -d) WINEPREFIX=$(mktemp -d)/wine
Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
export DISPLAY=:99
sleep 1
wineboot -i >/dev/null 2>&1
# Wine ships no Segoe UI/Arial; real Windows does.
cp /usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf "$WINEPREFIX/drive_c/windows/Fonts/arial.ttf"
cp /usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf "$WINEPREFIX/drive_c/windows/Fonts/arialbd.ttf"
wine "${EXE:-/src/dist/lowbattery.exe}" --demo >/tmp/app.log 2>&1 &
sleep "${WAIT:-12}"
import -window root /src/dist/smoke-devices.png
# Click the "Ajustes" tab of the main window, if found.
WID=$(xdotool search --name '^lowbattery$' | head -1)
if [ -n "$WID" ]; then
  eval "$(xdotool getwindowgeometry --shell "$WID")"
  xdotool mousemove $((X + WIDTH - 60)) $((Y + 40)) click 1
  sleep 2
  import -window root /src/dist/smoke-settings.png
fi
tail -20 /tmp/app.log
