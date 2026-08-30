#!/bin/bash
# Build a macOS application bundle. Public release mode also signs and notarizes.
set -euo pipefail

MODE="${1:-dev}"
case "$MODE" in
  dev|adhoc|release) ;;
  *) echo "usage: $0 [dev|adhoc|release]" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
ROOT="$PWD"
BUILD="$ROOT/build"
STAGE="$BUILD/stage.noindex"
APP="$STAGE/Kettle.app"
DMG="$BUILD/Kettle.dmg"
PKG="$BUILD/Kettle.pkg"
VERSION="$(awk -F'"' '/^version/{print $2; exit}' Cargo.toml)"
BUNDLE_ID="${KETTLE_BUNDLE_ID:-local.kettle.app}"
TOOLCHAIN="$(awk -F'"' '/^channel/{print $2; exit}' rust-toolchain.toml)"
RUSTC="$(rustup which --toolchain "$TOOLCHAIN" rustc)"
RUSTDOC="$(rustup which --toolchain "$TOOLCHAIN" rustdoc)"
TOOLCHAIN_BIN="$(dirname "$RUSTC")"
export PATH="$TOOLCHAIN_BIN:$PATH" RUSTC RUSTDOC
HOST="$("$RUSTC" -vV | awk '/^host:/{print $2}')"

if [[ "$MODE" == dev ]]; then
  TARGETS=("$HOST")
else
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
  INSTALLED="$(rustup target list --installed --toolchain "$TOOLCHAIN")"
  for target in "${TARGETS[@]}"; do
    grep -qx "$target" <<<"$INSTALLED" || {
      echo "missing Rust target $target; run: rustup target add $target" >&2
      exit 1
    }
  done
fi

for target in "${TARGETS[@]}"; do
  cargo build --release --target "$target" -p kettle -p kettle-askpass
done

rm -rf "$STAGE" "$BUILD/dmg.noindex" "$BUILD/AppIcon.iconset" "$DMG" "$PKG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in kettle kettle-askpass; do
  if [[ ${#TARGETS[@]} -eq 1 ]]; then
    cp "target/${TARGETS[0]}/release/$bin" "$APP/Contents/MacOS/$bin"
  else
    lipo -create -output "$APP/Contents/MacOS/$bin" \
      "target/${TARGETS[0]}/release/$bin" "target/${TARGETS[1]}/release/$bin"
  fi
  chmod 755 "$APP/Contents/MacOS/$bin"
done

cargo run --quiet --release -p icon-gen -- "$BUILD/icon.png"
ICONSET="$BUILD/AppIcon.iconset"
mkdir -p "$ICONSET"
for pixels in 16 32 128 256 512; do
  sips -z "$pixels" "$pixels" "$BUILD/icon.png" \
    --out "$ICONSET/icon_${pixels}x${pixels}.png" >/dev/null
  retina=$((pixels * 2))
  sips -z "$retina" "$retina" "$BUILD/icon.png" \
    --out "$ICONSET/icon_${pixels}x${pixels}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Kettle</string>
  <key>CFBundleDisplayName</key><string>Kettle</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>kettle</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict></plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" >/dev/null

if [[ "$MODE" == release ]]; then
  : "${KETTLE_CODESIGN_IDENTITY:?set KETTLE_CODESIGN_IDENTITY to a Developer ID Application identity}"
  : "${KETTLE_INSTALLER_IDENTITY:?set KETTLE_INSTALLER_IDENTITY to a Developer ID Installer identity}"
  : "${KETTLE_NOTARY_PROFILE:?set KETTLE_NOTARY_PROFILE to a notarytool keychain profile}"
  SIGN_ARGS=(--force --options runtime --timestamp --sign "$KETTLE_CODESIGN_IDENTITY")
else
  SIGN_ARGS=(--force --timestamp=none --sign -)
fi

# Sign code from the inside out. Never use codesign --deep.
for bin in kettle-askpass kettle; do
  codesign "${SIGN_ARGS[@]}" "$APP/Contents/MacOS/$bin"
  codesign --verify --strict --verbose=2 "$APP/Contents/MacOS/$bin"
done
codesign "${SIGN_ARGS[@]}" "$APP"
codesign --verify --strict --verbose=2 "$APP"

if [[ "$MODE" == dev ]]; then
  echo "development app (ad-hoc signed, not notarized): $APP"
  exit 0
fi

if [[ "$MODE" == release ]]; then
  ZIP="$BUILD/Kettle-notarization.zip"
  rm -f "$ZIP"
  ditto -c -k --keepParent "$APP" "$ZIP"
  xcrun notarytool submit "$ZIP" --keychain-profile "$KETTLE_NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
fi

if [[ "$MODE" == release ]]; then
  productbuild --sign "$KETTLE_INSTALLER_IDENTITY" \
    --component "$APP" /Applications "$PKG"
  pkgutil --check-signature "$PKG"
  xcrun notarytool submit "$PKG" --keychain-profile "$KETTLE_NOTARY_PROFILE" --wait
  xcrun stapler staple "$PKG"
  xcrun stapler validate "$PKG"
  spctl --assess --type install --verbose=2 "$PKG"
else
  productbuild --component "$APP" /Applications "$PKG"
fi
pkgutil --payload-files "$PKG" | grep -q 'Kettle.app/Contents/MacOS/kettle$'

mkdir -p "$BUILD/dmg.noindex"
cp -R "$APP" "$BUILD/dmg.noindex/"
ln -s /Applications "$BUILD/dmg.noindex/Applications"
hdiutil create -volname Kettle -srcfolder "$BUILD/dmg.noindex" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$BUILD/dmg.noindex"

if [[ "$MODE" == release ]]; then
  codesign "${SIGN_ARGS[@]}" "$DMG"
  codesign --verify --strict --verbose=2 "$DMG"
  xcrun notarytool submit "$DMG" --keychain-profile "$KETTLE_NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type execute --verbose=2 "$APP"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
  echo "notarized release app: $APP"
  echo "notarized release dmg: $DMG"
  echo "notarized release pkg: $PKG"
else
  echo "ad-hoc app (not notarized; local testing only): $APP"
  echo "ad-hoc dmg (not notarized; local testing only): $DMG"
  echo "unsigned pkg containing the ad-hoc app (local testing only): $PKG"
fi
