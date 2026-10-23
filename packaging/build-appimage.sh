#!/usr/bin/env bash
# Crea dist/tplink-battery-monitor-x86_64.AppImage dal binario Rust statico (musl).
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET=x86_64-unknown-linux-musl
TOOLS=target/appimage-tools
APPDIR=target/AppDir
APPIMAGETOOL="$TOOLS/appimagetool-x86_64.AppImage"

rustup target add "$TARGET" >/dev/null
cargo build --release --target "$TARGET"

if [ ! -x "$APPIMAGETOOL" ]; then
    mkdir -p "$TOOLS"
    curl -fL -o "$APPIMAGETOOL" \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x "$APPIMAGETOOL"
fi

rm -rf "$APPDIR"
install -Dm755 "target/$TARGET/release/tplink-battery-monitor" "$APPDIR/usr/bin/tplink-battery-monitor"
install -Dm644 packaging/tplink-battery-monitor.desktop "$APPDIR/tplink-battery-monitor.desktop"
install -Dm644 assets/icon.png "$APPDIR/tplink-battery-monitor.png"
ln -s usr/bin/tplink-battery-monitor "$APPDIR/AppRun"
ln -s tplink-battery-monitor.png "$APPDIR/.DirIcon"

mkdir -p dist
# Rimuovere invece di sovrascrivere: il file puo' essere in esecuzione.
rm -f dist/tplink-battery-monitor-x86_64.AppImage
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream \
    "$APPDIR" dist/tplink-battery-monitor-x86_64.AppImage
