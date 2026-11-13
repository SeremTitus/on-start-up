#!/usr/bin/env bash
set -euo pipefail

# on-start-up installer for Linux, macOS, and Windows (Git Bash/MSYS2/Cygwin)
# Usage: curl -fsSL https://on-start-up.seremtitus.co.ke/install.sh | bash

APP=on-start-up
APP_NAME="On Start Up"
GITHUB_REPO="SeremTitus/on-start-up"
INSTALL_DIR="${ON_START_UP_INSTALL_DIR:-$HOME/.on-start-up/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}✓${NC} $*"; }
warn()  { echo -e "${YELLOW}!${NC} $*"; }
error() { echo -e "${RED}✗${NC} $*" >&2; exit 1; }

# Detect OS
raw_os=$(uname -s)
case "$raw_os" in
    Darwin*) os="macos" ;;
    Linux*)  os="linux" ;;
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *)       error "Unsupported OS: $raw_os" ;;
esac

# Detect WSL
if grep -qi microsoft /proc/version 2>/dev/null; then
    warn "WSL detected. Use the PowerShell command if a Windows install was intended."
fi

# Detect architecture (matches release naming)
arch=$(uname -m)
case "$arch" in
    x86_64|amd64)   arch="x86_64" ;;
    aarch64|arm64)  arch="aarch64" ;;
    *)              error "Unsupported architecture: $arch" ;;
esac

combo="$os-$arch"
case "$combo" in
    linux-x86_64|macos-x86_64|macos-aarch64|windows-x86_64) ;;
    *) error "Unsupported OS/Arch: $os/$arch (release builds: linux-x86_64, macos-x86_64, macos-aarch64, windows-x86_64)" ;;
esac

archive_ext=".tar.gz"
[ "$os" = "windows" ] && archive_ext=".zip"

archive="$APP-$os-$arch$archive_ext"
url="https://github.com/$GITHUB_REPO/releases/latest/download/$archive"

info "Downloading $archive ..."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL --retry 3 -o "$tmp/$archive" "$url" \
    || error "Download failed: $url (is there a release for $os-$arch?)"

info "Extracting ..."
case "$archive_ext" in
    .zip) unzip -oq "$tmp/$archive" -d "$tmp/out" ;;
    *)    mkdir -p "$tmp/out" && tar -xzf "$tmp/$archive" -C "$tmp/out" ;;
esac

info "Installing to $INSTALL_DIR ..."
mkdir -p "$INSTALL_DIR"
# Binaries: on-start-up (UI) + on-start-up-server (background service) + on-start-up-error-popup
for bin in "$APP" "$APP-server" "$APP-error-popup"; do
    src=""
    [ -f "$tmp/out/$bin" ] && src="$tmp/out/$bin"
    [ -f "$tmp/out/$bin.exe" ] && src="$tmp/out/$bin.exe"
    [ -n "$src" ] || error "Missing $bin in the archive"
    cp -f "$src" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/$(basename "$src")" 2>/dev/null || true
    info "  installed $(basename "$src")"
done

# Launcher / Start Menu entry and uninstall script
LAUNCHER=""
case "$os" in
    linux)
        apps_dir="$HOME/.local/share/applications"
        mkdir -p "$apps_dir"
        LAUNCHER="$apps_dir/$APP.desktop"
        cat > "$LAUNCHER" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
Comment=Auto-launch process manager
Exec=$INSTALL_DIR/$APP
Terminal=false
Categories=Utility;
EOF
        info "Added launcher: $LAUNCHER"
        ;;
    macos)
        apps_dir="$HOME/Applications"
        mkdir -p "$apps_dir"
        LAUNCHER="$apps_dir/$APP_NAME"
        ln -sf "$INSTALL_DIR/$APP" "$LAUNCHER"
        info "Added launcher: $LAUNCHER"
        ;;
    windows)
        # Git Bash/MSYS2/Cygwin: best-effort Start Menu shortcut via PowerShell
        if command -v powershell.exe >/dev/null 2>&1; then
            sm_dir="$APPDATA/Microsoft/Windows/Start Menu/Programs"
            LAUNCHER="$sm_dir/$APP_NAME.lnk"
            mkdir -p "$sm_dir" 2>/dev/null || true
            win_target="$(cygpath -w "$INSTALL_DIR/$APP.exe" 2>/dev/null || echo "$INSTALL_DIR\\$APP.exe")"
            win_workdir="$(cygpath -w "$INSTALL_DIR" 2>/dev/null || echo "$INSTALL_DIR")"
            win_lnk="$(cygpath -w "$LAUNCHER" 2>/dev/null || echo "$LAUNCHER")"
            powershell.exe -NoProfile -Command "\$ws=New-Object -ComObject WScript.Shell;\$s=\$ws.CreateShortcut('$win_lnk');\$s.TargetPath='$win_target';\$s.WorkingDirectory='$win_workdir';\$s.Description='$APP_NAME';\$s.IconLocation='$win_target,0';\$s.Save()" >/dev/null 2>&1 \
                && info "Added Start Menu shortcut: $LAUNCHER" \
                || warn "Could not create Start Menu shortcut (app installed at $INSTALL_DIR)"
        else
            warn "PowerShell not found; skipping Start Menu shortcut (app installed at $INSTALL_DIR)"
        fi
        ;;
esac

# Uninstall script
cat > "$INSTALL_DIR/uninstall.sh" <<'UNINSTALL'
#!/usr/bin/env bash
set -euo pipefail
APP="__APP__"
APP_NAME="__APP_NAME__"
INSTALL_DIR="__INSTALL_DIR__"
LAUNCHER="__LAUNCHER__"

# Stop running app/service (best-effort)
if command -v curl >/dev/null 2>&1; then
  curl -fsS -X POST http://127.0.0.1:46231/api/shutdown >/dev/null 2>&1 || true
fi
sleep 1
pkill -f "$INSTALL_DIR/$APP-server" >/dev/null 2>&1 || true
pkill -f "$INSTALL_DIR/$APP-error-popup" >/dev/null 2>&1 || true
pkill -f "$INSTALL_DIR/$APP" >/dev/null 2>&1 || true

# Remove launcher
[ -n "$LAUNCHER" ] && [ -e "$LAUNCHER" ] && rm -f "$LAUNCHER"

# Remove binaries and this script
rm -f "$INSTALL_DIR/$APP" "$INSTALL_DIR/$APP.exe" "$INSTALL_DIR/$APP-server" "$INSTALL_DIR/$APP-server.exe" "$INSTALL_DIR/$APP-error-popup" "$INSTALL_DIR/$APP-error-popup.exe" "$INSTALL_DIR/uninstall.sh"

# Remove install dir if empty
rmdir "$INSTALL_DIR" 2>/dev/null || true
rmdir "$(dirname "$INSTALL_DIR")" 2>/dev/null || true

echo "✓ On Start Up uninstalled."
UNINSTALL
# Fill placeholders
sed -i.bak \
  -e "s|__APP__|$APP|g" \
  -e "s|__APP_NAME__|$APP_NAME|g" \
  -e "s|__INSTALL_DIR__|$INSTALL_DIR|g" \
  -e "s|__LAUNCHER__|$LAUNCHER|g" \
  "$INSTALL_DIR/uninstall.sh" 2>/dev/null || \
sed -i \
  -e "s|__APP__|$APP|g" \
  -e "s|__APP_NAME__|$APP_NAME|g" \
  -e "s|__INSTALL_DIR__|$INSTALL_DIR|g" \
  -e "s|__LAUNCHER__|$LAUNCHER|g" \
  "$INSTALL_DIR/uninstall.sh"
rm -f "$INSTALL_DIR/uninstall.sh.bak"
chmod +x "$INSTALL_DIR/uninstall.sh"
info "  installed uninstall.sh"

info "Done. Launch '$APP_NAME' from your applications menu."
echo
echo "  It opens the UI, starts the background service and registers itself"
echo "  to launch at login (toggle in the UI: 'start with system')."
echo "  To uninstall: bash \"$INSTALL_DIR/uninstall.sh\""
