#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

prefix="${PREFIX:-$HOME/.local}"
binary="${CARGO_TARGET_DIR:-target}/release/safeincloud-linux"
if [[ ! -x "$binary" ]]; then
  cargo build --locked --release
fi
install -d "$prefix/bin" "$prefix/share/applications" \
  "$prefix/share/icons/hicolor/scalable/apps" "$prefix/share/safeincloud-linux"
temporary="$(mktemp "$prefix/bin/.safeincloud-linux.XXXXXX")"
trap 'if [[ -f "$temporary" ]]; then unlink "$temporary"; fi' EXIT
install -m 755 "$binary" "$temporary"
mv -f "$temporary" "$prefix/bin/safeincloud-linux"
install -m 644 assets/icon.svg "$prefix/share/icons/hicolor/scalable/apps/safeincloud-linux.svg"
install -m 644 LICENSE THIRD_PARTY_LICENSE THIRD_PARTY_NOTICES.md README.md Cargo.toml Cargo.lock \
  "$prefix/share/safeincloud-linux/"
cp -R assets "$prefix/share/safeincloud-linux/"
cp -R docs "$prefix/share/safeincloud-linux/"
install -d "$prefix/share/safeincloud-linux/tests"
cp -R tests/fixtures "$prefix/share/safeincloud-linux/tests/"
cat > "$prefix/share/applications/io.github.vassiljevnikita.SafeInCloudLinux.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=SafeInCloud Linux
Comment=Nativer Passwortmanager mit Rust und GTK4
Exec="$prefix/bin/safeincloud-linux" %f
Icon=$prefix/share/icons/hicolor/scalable/apps/safeincloud-linux.svg
Terminal=false
Categories=System;Security;
StartupNotify=true
StartupWMClass=io.github.vassiljevnikita.SafeInCloudLinux
EOF
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$prefix/share/applications" || true
fi
printf 'Installiert: %s\n' "$prefix/bin/safeincloud-linux"
