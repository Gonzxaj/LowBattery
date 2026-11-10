#!/usr/bin/env bash
# Usage: ./build.sh [test|check|release|installer|shell]   (installer = MSIX)
# Everything runs inside the lowbattery-build image; the cargo registry and
# target dir are cached on the host so rebuilds are incremental.
set -euo pipefail
cd "$(dirname "$0")"
IMG=lowbattery-build
docker image inspect "$IMG" >/dev/null 2>&1 || docker build -t "$IMG" docker/
run() {
  docker run --rm -u "$(id -u):$(id -g)" \
    -e CARGO_HOME=/src/.cache/cargo -v "$PWD":/src -w /src "$IMG" "$@"
}
WIN=x86_64-pc-windows-gnu
case "${1:-release}" in
  test)      run cargo test -p battery-core -p lowbattery --lib --bins ;;
  check)     run cargo clippy --target $WIN --all-targets -- -D warnings ;;
  release)   run cargo build --release --target $WIN -p lowbattery
             mkdir -p dist && cp target/$WIN/release/lowbattery.exe dist/
             ls -lh dist/lowbattery.exe ;;
  installer) "$0" release
             docker image inspect lowbattery-msix >/dev/null 2>&1 || docker build -t lowbattery-msix -f docker/Dockerfile.msix docker/
             VER="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2).0"
             docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/src lowbattery-msix bash /src/docker/pack-msix.sh "$VER"
             ls -lh dist/*.msix dist/*.cer ;;
  shell)     docker run --rm -it -v "$PWD":/src -w /src "$IMG" bash ;;
  *) echo "usage: $0 [test|check|release|installer|shell]"; exit 1 ;;
esac
