#!/usr/bin/env bash
# Runs inside lowbattery-msix (see docker/Dockerfile.msix). Produces:
#   dist/lowbattery-<ver>.msix            unsigned: upload this to Partner Center (the Store signs it)
#   dist/lowbattery-<ver>-sideload.msix   signed with a local dev cert, to install on your own PC
#   dist/lowbattery-dev.cer               that cert; trust it once to sideload
# Identity comes from installer/msix.env (copy msix.env.example) or the environment.
set -euo pipefail
cd /src
[ -f installer/msix.env ] && . installer/msix.env
: "${MSIX_NAME:=LowBattery.Monitor}"
: "${MSIX_PUBLISHER:=CN=LowBattery Dev}"
: "${MSIX_PUBLISHER_DISPLAY:=LowBattery}"
VERSION="$1"   # x.y.z.0 (the Store requires the 4th part to be 0)

STAGE=$(mktemp -d)
mkdir -p "$STAGE/Assets"
cp dist/lowbattery.exe "$STAGE/"

# Logos from assets/: emblem for the small sizes, full hexagon for tiles.
convert assets/emblem-tile.png -resize 44x44 "$STAGE/Assets/Square44x44Logo.png"
convert assets/emblem-tile.png -resize 50x50 "$STAGE/Assets/StoreLogo.png"
convert assets/logo-square.png -resize 150x150 "$STAGE/Assets/Square150x150Logo.png"
convert assets/logo-square.png -resize 150x150 -background none -gravity center -extent 310x150 "$STAGE/Assets/Wide310x150Logo.png"

# XML-escape identity values, then escape for the sed replacement (& | \ are special there).
esc() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g' \
    | sed -e 's/[&|\\]/\\&/g'
}
sed -e "s|@NAME@|$(esc "$MSIX_NAME")|" \
    -e "s|@PUBLISHER@|$(esc "$MSIX_PUBLISHER")|" \
    -e "s|@PUBLISHER_DISPLAY@|$(esc "$MSIX_PUBLISHER_DISPLAY")|" \
    -e "s|@VERSION@|$VERSION|" \
    installer/AppxManifest.xml > "$STAGE/AppxManifest.xml"

OUT="dist/lowbattery-$VERSION.msix"
rm -f "$OUT"
makemsix pack -d "$STAGE" -p "$OUT"
echo "store package: $OUT"

# Sideload copy: the cert subject must equal the manifest Publisher.
CERT_DIR=.cache/msix-dev-cert
mkdir -p "$CERT_DIR"
if [ ! -f "$CERT_DIR/dev.pfx" ] || [ "$(cat "$CERT_DIR/subject" 2>/dev/null)" != "$MSIX_PUBLISHER" ]; then
  # "CN=A, O=B" -> "/CN=A/O=B" for openssl
  SUBJ="/$(printf '%s' "$MSIX_PUBLISHER" | sed 's/, */\//g')"
  openssl req -x509 -newkey rsa:3072 -sha256 -days 1095 -nodes -subj "$SUBJ" \
    -addext "basicConstraints=critical,CA:FALSE" -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=codeSigning" \
    -keyout "$CERT_DIR/dev.key" -out "$CERT_DIR/dev.crt" 2>/dev/null
  openssl pkcs12 -export -inkey "$CERT_DIR/dev.key" -in "$CERT_DIR/dev.crt" -out "$CERT_DIR/dev.pfx" -passout pass:
  printf '%s' "$MSIX_PUBLISHER" > "$CERT_DIR/subject"
fi
openssl x509 -in "$CERT_DIR/dev.crt" -outform der -out dist/lowbattery-dev.cer
SIDE="dist/lowbattery-$VERSION-sideload.msix"
rm -f "$SIDE"
osslsigncode sign -pkcs12 "$CERT_DIR/dev.pfx" -pass "" -h sha256 -in "$OUT" -out "$SIDE" >/dev/null
echo "sideload package: $SIDE (+ dist/lowbattery-dev.cer)"
rm -rf "$STAGE"
