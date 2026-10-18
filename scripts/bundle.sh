#!/bin/sh
# Build a release binary and wrap it in Bluemeth.app (ad-hoc signed).
#
#   scripts/bundle.sh            -> target/Bluemeth.app
#   scripts/bundle.sh --install  -> also copies to /Applications
set -eu

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/Bluemeth.app
BIN=target/release/bluemeth-app
BUNDLE_ID=com.alexyavo.bluemeth
ICON=assets/AppIcon.icns

cargo build --release -p bluemeth-app

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Bluemeth"
[ -f "$ICON" ] || { echo "bundle.sh: $ICON is missing; run scripts/make-icon.sh" >&2; exit 1; }
cp "$ICON" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>Bluemeth</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Bluemeth</string>
	<key>CFBundleDisplayName</key>
	<string>Bluemeth</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
	<key>LSMinimumSystemVersion</key>
	<string>13.0</string>
	<key>LSUIElement</key>
	<true/>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.utilities</string>
	<key>NSHumanReadableCopyright</key>
	<string>MIT License</string>
	<key>NSSupportsAutomaticTermination</key>
	<false/>
	<key>NSSupportsSuddenTermination</key>
	<true/>
</dict>
</plist>
PLIST
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Ad-hoc signature: stable identity for SMAppService and TCC without a
# developer certificate. Replace "-" with your signing identity to notarise.
codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"

echo "built $APP ($(du -sh "$APP" | cut -f1))"

if [ "${1:-}" = "--install" ]; then
    rm -rf /Applications/Bluemeth.app
    cp -R "$APP" /Applications/Bluemeth.app
    touch /Applications/Bluemeth.app   # make Finder refresh its icon cache
    echo "installed /Applications/Bluemeth.app"
    if pgrep -xq Bluemeth; then
        echo "note: Bluemeth is running; quit and relaunch it to pick up this build"
    fi
fi
