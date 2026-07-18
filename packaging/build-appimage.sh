#!/usr/bin/env bash
set -euo pipefail

# Build AppImage for SysCtrl
# Requires: cargo, tauri-cli, appimagetool, linuxdeploy, linuxdeploy-plugin-gtk

APP_NAME="SysCtrl"
VERSION="${1:-$(grep '^version' ../src-tauri/Cargo.toml | head -1 | cut -d'"' -f2)}"
ARCH="x86_64"
OUT_DIR="out"
APPDIR="${OUT_DIR}/${APP_NAME}.AppDir"

echo "Building ${APP_NAME} ${VERSION}..."

# 1. Build Tauri app (release)
cd ..
cargo tauri build --target x86_64-unknown-linux-gnu

# 2. Build sysctl-helper
cd sysctl-helper
cargo build --release --target x86_64-unknown-linux-gnu
cd ..

# 3. Create AppDir structure
rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin"
mkdir -p "${APPDIR}/usr/share/applications"
mkdir -p "${APPDIR}/usr/share/icons/hicolor/256x256/apps"
mkdir -p "${APPDIR}/usr/share/polkit-1/actions"
mkdir -p "${APPDIR}/usr/lib/udev/rules.d"

# 4. Copy binaries
cp "src-tauri/target/x86_64-unknown-linux-gnu/release/sysctrl" "${APPDIR}/usr/bin/"
cp "sysctl-helper/target/x86_64-unknown-linux-gnu/release/sysctl-helper" "${APPDIR}/usr/bin/"

# 5. Desktop entry
cat > "${APPDIR}/usr/share/applications/${APP_NAME}.desktop" <<EOF
[Desktop Entry]
Name=${APP_NAME}
Comment=Linux system monitor & fan control
Exec=sysctrl
Icon=${APP_NAME}
Terminal=false
Type=Application
Categories=System;Monitor;Hardware;
StartupNotify=true
EOF

# 6. Icon (placeholder - replace with real icon)
# cp ../icons/icon.png "${APPDIR}/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png"
# For now create a minimal PNG via ImageMagick if available
if command -v convert &> /dev/null; then
    convert -size 256x256 xc:#2d3748 -fill white -gravity center -pointsize 48 -annotate +0+0 "SysCtrl" "${APPDIR}/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png"
else
    echo "Warning: ImageMagick not found, skipping icon generation"
fi

# 7. Polkit policy
cp packaging/com.sysctl.helper.policy "${APPDIR}/usr/share/polkit-1/actions/"

# 8. Udev rules
cp packaging/99-sysctl.rules "${APPDIR}/usr/lib/udev/rules.d/"

# 9. AppRun entry point
cat > "${APPDIR}/AppRun" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

APPDIR="$(dirname "$(readlink -f "$0")")"
export PATH="${APPDIR}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${APPDIR}/usr/lib:${LD_LIBRARY_PATH:-}"

# Ensure polkit agent is available for pkexec
if [ -z "${PKEXEC_UID:-}" ] && [ "$(id -u)" -ne 0 ]; then
    # Running as user - normal operation
    exec "${APPDIR}/usr/bin/sysctrl" "$@"
else
    # Fallback (shouldn't happen for GUI app)
    exec "${APPDIR}/usr/bin/sysctrl" "$@"
fi
EOF
chmod +x "${APPDIR}/AppRun"

# 10. Build AppImage
cd "${OUT_DIR}"
ARCH=${ARCH} appimagetool --comp zstd "${APP_NAME}.AppDir" "${APP_NAME}-${VERSION}-${ARCH}.AppImage"

echo "Done: ${OUT_DIR}/${APP_NAME}-${VERSION}-${ARCH}.AppImage"