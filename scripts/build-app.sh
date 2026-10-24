#!/bin/sh
set -eu
if [ "$(uname -s)" != Darwin ]; then
  printf 'App packaging requires macOS\n' >&2
  exit 1
fi
build_dir="${CARGO_TARGET_DIR:-target}"
help_assets=$(python3 -B scripts/help_assets.py)
target=$(rustc --print host-tuple)
. scripts/build-features.sh
cargo build --profile app --locked -p vpdf-gui --target "$target" "$@"
binary="$build_dir/$target/app/vpdf-gui"
sh scripts/check-macos.sh "$binary"
release_dir="$build_dir/dist"
mkdir -p "$release_dir"
staged=$(mktemp -d "$release_dir/.app.XXXXXX")
trap 'rm -rf "$staged"' EXIT
trap 'exit 1' HUP INT TERM
app="$staged/image/Vpdf.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$binary" "$app/Contents/MacOS/vpdf-gui"
cp LICENSE LICENSE-AGPL "$app/Contents/Resources/"
cp "$help_assets/font-notices.txt" "$app/Contents/Resources/"
cp docs/assets/vpdf.icns "$app/Contents/Resources/Vpdf.icns"
version=$(cargo metadata --locked --no-deps --format-version 1 |
  python3 -c '
import json, sys
packages = json.load(sys.stdin)["packages"]
print(next(p["version"] for p in packages if p["name"] == "vpdf-gui"))
')
minimum=$(sh scripts/macos-minimum.sh "$binary")
test -n "$version"
test -n "$minimum"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>vpdf-gui</string>
  <key>CFBundleIdentifier</key>
  <string>io.github.letientai299.vpdf</string>
  <key>CFBundleName</key>
  <string>Vpdf</string>
  <key>CFBundleDisplayName</key>
  <string>Vpdf</string>
  <key>CFBundleIconFile</key>
  <string>Vpdf.icns</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key>
      <string>PDF document</string>
      <key>CFBundleTypeRole</key>
      <string>Viewer</string>
      <key>LSHandlerRank</key>
      <string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>com.adobe.pdf</string>
      </array>
    </dict>
  </array>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>LSMinimumSystemVersion</key>
  <string>$minimum</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict "$app"
ln -s /Applications "$staged/image/Applications"
hdiutil create -quiet -srcfolder "$staged/image" -volname Vpdf \
  -fs HFS+ -format UDRW "$staged/image.dmg"
hdiutil convert -quiet "$staged/image.dmg" -format ULFO -o "$staged/Vpdf.dmg"
hdiutil verify -quiet "$staged/Vpdf.dmg"
python3 -B scripts/replace_app.py "$app" "$release_dir/Vpdf.app"
mv -f "$staged/Vpdf.dmg" "$release_dir/Vpdf.dmg"
"$release_dir/Vpdf.app/Contents/MacOS/vpdf-gui" --version >/dev/null
printf 'App: %s/Vpdf.app\nDMG: %s/Vpdf.dmg\n' "$release_dir" "$release_dir"
