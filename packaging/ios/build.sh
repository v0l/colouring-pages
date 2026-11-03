#!/usr/bin/env bash
# Builds ColouringPages.app for the iOS simulator (default) or a device.
# Device: TARGET=aarch64-apple-ios SIGN_IDENTITY="Apple Development: ..." PROVISION=path.mobileprovision
# The settings in defaults.json (or COLOURING_DEFAULTS) are baked into the binary.
set -euo pipefail
cd "$(dirname "$0")/../.."
TARGET=${TARGET:-aarch64-apple-ios-sim}
BUNDLE_ID=${BUNDLE_ID:-io.v0l.colouringpages}
BUILD=${BUILD:-$(date +%Y%m%d%H%M)}
APP=target/ios/ColouringPages.app
cargo build --release --target "$TARGET" --bin colouring-pages
rm -rf "$APP"
mkdir -p "$APP"
cp "target/$TARGET/release/colouring-pages" "$APP/ColouringPages"
sed -e "s/__BUNDLE_ID__/$BUNDLE_ID/" -e "s/__BUILD__/$BUILD/" packaging/ios/Info.plist > "$APP/Info.plist"
platform=iphonesimulator
[ "$TARGET" = aarch64-apple-ios ] && platform=iphoneos
icons=target/ios/Assets.xcassets/AppIcon.appiconset
rm -rf target/ios/Assets.xcassets
mkdir -p "$icons"
images=""
for spec in iphone:20:2 iphone:20:3 iphone:29:2 iphone:29:3 iphone:40:2 iphone:40:3 iphone:60:2 iphone:60:3 ipad:20:1 ipad:20:2 ipad:29:1 ipad:29:2 ipad:40:1 ipad:40:2 ipad:76:1 ipad:76:2 ipad:83.5:2; do
  IFS=: read -r idiom size scale <<<"$spec"
  px=$(python3 -c "print(round($size*$scale))")
  name="icon-$idiom-$size@${scale}x.png"
  sips -z "$px" "$px" packaging/ios/icon-1024.png --out "$icons/$name" >/dev/null
  images="$images{\"filename\":\"$name\",\"idiom\":\"$idiom\",\"scale\":\"${scale}x\",\"size\":\"${size}x${size}\"},"
done
cp packaging/ios/icon-1024.png "$icons/icon-marketing.png"
echo "{\"images\":[$images{\"filename\":\"icon-marketing.png\",\"idiom\":\"ios-marketing\",\"scale\":\"1x\",\"size\":\"1024x1024\"}],\"info\":{\"author\":\"xcode\",\"version\":1}}" > "$icons/Contents.json"
echo '{"info":{"author":"xcode","version":1}}' > target/ios/Assets.xcassets/Contents.json
xcrun actool target/ios/Assets.xcassets --compile "$APP" --platform "$platform" --minimum-deployment-target 15.0 \
  --app-icon AppIcon --output-partial-info-plist target/ios/icons.plist --target-device iphone --target-device ipad >/dev/null
/usr/libexec/PlistBuddy -c "Merge target/ios/icons.plist" "$APP/Info.plist" 2>/dev/null || true
echo "APPL????" > "$APP/PkgInfo"
if [ "$TARGET" = aarch64-apple-ios ]; then
  sdk_build=$(xcrun --sdk iphoneos --show-sdk-build-version)
  sdk_ver=$(xcrun --sdk iphoneos --show-sdk-version)
  for kv in "DTPlatformName:iphoneos" "DTSDKName:iphoneos$sdk_ver" "DTSDKBuild:$sdk_build" "DTPlatformVersion:$sdk_ver" "DTPlatformBuild:$sdk_build"; do
    /usr/libexec/PlistBuddy -c "Add :${kv%%:*} string ${kv#*:}" "$APP/Info.plist" 2>/dev/null || true
  done
  cp "${PROVISION:?set PROVISION to a profile for $BUNDLE_ID}" "$APP/embedded.mobileprovision"
  security cms -D -i "$PROVISION" > target/ios/profile.plist
  plutil -extract Entitlements xml1 -o target/ios/entitlements.plist target/ios/profile.plist
  codesign --force --timestamp=none --sign "${SIGN_IDENTITY:?set SIGN_IDENTITY}" \
    ${KEYCHAIN:+--keychain "$KEYCHAIN"} --entitlements target/ios/entitlements.plist "$APP"
else
  codesign --force --sign - "$APP"
fi
echo "$APP"
