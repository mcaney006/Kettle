#!/bin/bash
# Build Kettle.app (universal) and a distributable Kettle.dmg.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
BUILD="$ROOT/build"
# Staged inside a `.noindex` directory: Spotlight skips those by convention, so
# the build copy is never indexed and never registered with LaunchServices.
# Without this, every build re-registers a second "Kettle" and it shows up in
# Spotlight and Launchpad alongside the one in /Applications. Unregistering it
# afterwards does not stick -- the next reindex puts it right back.
STAGE="$BUILD/stage.noindex"
APP="$STAGE/Kettle.app"
VERSION="$(awk -F'"' '/^version/{print $2; exit}' Cargo.toml)"

echo "==> building universal binaries (v$VERSION)"
for TARGET in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --target "$TARGET"
  cargo build --release --target "$TARGET" -p kettle-askpass
done

rm -rf "$STAGE" "$BUILD/dmg" "$BUILD/dmg.noindex" "$BUILD/Kettle.dmg" "$BUILD/Kettle.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

for BIN in kettle kettle-askpass; do
  lipo -create -output "$APP/Contents/MacOS/$BIN" \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN"
  chmod +x "$APP/Contents/MacOS/$BIN"
done

echo "==> icon"
cargo run -q -p icon-gen --release -- "$BUILD/icon.png"
ICONSET="$BUILD/AppIcon.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
for sz in 16 32 128 256 512; do
  sips -z $sz $sz "$BUILD/icon.png" --out "$ICONSET/icon_${sz}x${sz}.png" >/dev/null
  sips -z $((sz*2)) $((sz*2)) "$BUILD/icon.png" --out "$ICONSET/icon_${sz}x${sz}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Kettle</string>
  <key>CFBundleDisplayName</key><string>Kettle</string>
  <key>CFBundleIdentifier</key><string>local.kettle.app</string>
  <key>CFBundleExecutable</key><string>kettle</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

echo "==> signing (ad-hoc)"
# Ad-hoc: no Developer ID here, so the app is signed but not notarized.
codesign --force --deep --sign - --timestamp=none "$APP"
codesign --verify --strict --verbose=1 "$APP"

echo "==> dmg"
# Also .noindex: this staging copy exists only for the moments hdiutil needs
# it, but that is long enough for LaunchServices to register a second Kettle,
# and the registration outlives the directory.
mkdir -p "$BUILD/dmg.noindex"
cp -R "$APP" "$BUILD/dmg.noindex/"
ln -s /Applications "$BUILD/dmg.noindex/Applications"
hdiutil create -volname "Kettle" -srcfolder "$BUILD/dmg.noindex" -ov -format UDZO \
  "$BUILD/Kettle.dmg" >/dev/null
rm -rf "$BUILD/dmg.noindex"

# Belt and braces: .noindex stops future indexing, this drops any registration
# an earlier build already created.
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
[ -x "$LSREGISTER" ] && "$LSREGISTER" -u "$APP" 2>/dev/null || true

echo
echo "app: $APP"
echo "dmg: $BUILD/Kettle.dmg"
lipo -archs "$APP/Contents/MacOS/kettle"
du -h "$BUILD/Kettle.dmg" | cut -f1
