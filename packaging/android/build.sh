#!/usr/bin/env bash
# Build the Android APK: the Rust library through cargo-ndk, then the Kotlin activity,
# manifest and signature on top. Output: target/android/colouring-pages.apk
#   ANDROID_HOME  the SDK (default ~/Android/Sdk)
#   ABI           arm64-v8a (default, tablets) or x86_64 (the emulator)
#   KOTLIN_HOME   a kotlinc install (default: downloaded to ~/.cache/agentbench-android)
#   KEYSTORE / KEYSTORE_PASS / KEY_PASS / KEY_ALIAS   to sign a release, the debug key otherwise
#   RELEASE=1     a release build: not debuggable, built with --release
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD
SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
export ANDROID_HOME=$SDK
export ANDROID_NDK_ROOT=${ANDROID_NDK_ROOT:-$(ls -d "$SDK"/ndk/* | sort -V | tail -1)}
export ANDROID_NDK_HOME=$ANDROID_NDK_ROOT
BT=$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)
JAR=$(ls -d "$SDK"/platforms/android-3[4-9]* | sort -V | tail -1)/android.jar
KOTLIN_HOME=${KOTLIN_HOME:-$HOME/.cache/agentbench-android/kotlinc}
KOTLIN_VERSION=2.4.21
ABI=${ABI:-arm64-v8a}
case $ABI in
    arm64-v8a|x86_64) ;;
    *) echo "unknown ABI $ABI"; exit 1 ;;
esac
if [ -n "${RELEASE:-}" ]; then
    FLAG=--release; DEBUGGABLE=false
else
    FLAG=; DEBUGGABLE=true
fi
if [ ! -x "$KOTLIN_HOME/bin/kotlinc" ]; then
    mkdir -p "$(dirname "$KOTLIN_HOME")"
    curl -fsSL -o /tmp/kotlinc.zip \
        "https://github.com/JetBrains/kotlin/releases/download/v$KOTLIN_VERSION/kotlin-compiler-$KOTLIN_VERSION.zip"
    unzip -q -o /tmp/kotlinc.zip -d "$(dirname "$KOTLIN_HOME")"
fi
OUT=$ROOT/target/android
rm -rf "$OUT"; mkdir -p "$OUT/dex"

echo "== rust ($ABI)"
JNI=$ROOT/target/android-jni
rm -rf "$JNI"
cargo ndk --platform 26 -t "$ABI" -o "$JNI" build --lib --no-default-features $FLAG
LIB=$JNI/$ABI/libcolouring_pages.so
[ -f "$LIB" ] || { echo "no library built"; exit 1; }

echo "== kotlin"
"$KOTLIN_HOME/bin/kotlinc" -nowarn -jvm-target 17 -cp "$JAR" -d "$OUT/classes.jar" android/src/io/v0l/colouringpages/*.kt
"$BT/d8" --min-api 26 --lib "$JAR" --output "$OUT/dex" "$OUT/classes.jar" "$KOTLIN_HOME/lib/kotlin-stdlib.jar"

echo "== resources and manifest"
sed "s/@DEBUGGABLE@/$DEBUGGABLE/" packaging/android/AndroidManifest.xml > "$OUT/AndroidManifest.xml"
"$BT/aapt2" compile --dir android/res -o "$OUT/res.zip"
VERSION=$(cargo pkgid | sed 's/.*[#@]//')
IFS=. read -r MAJOR MINOR PATCH <<<"${VERSION%%[-+]*}"
CODE=$((MAJOR * 10000 + MINOR * 100 + PATCH))
"$BT/aapt2" link -o "$OUT/base.apk" -I "$JAR" --manifest "$OUT/AndroidManifest.xml" \
    --min-sdk-version 26 --target-sdk-version 34 --version-code "$CODE" --version-name "$VERSION" "$OUT/res.zip"

echo "== package"
python3 - "$OUT" "$LIB" "$ABI" <<'PY'
import sys, zipfile
out, lib, abi = sys.argv[1:4]
with zipfile.ZipFile(f"{out}/base.apk", "a") as z:
    z.write(f"{out}/dex/classes.dex", "classes.dex", zipfile.ZIP_DEFLATED)
    z.write(lib, f"lib/{abi}/libcolouring_pages.so", zipfile.ZIP_STORED)
PY
"$BT/zipalign" -f -p 4 "$OUT/base.apk" "$OUT/aligned.apk"
if [ -n "${KEYSTORE:-}" ]; then
    "$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass "pass:${KEYSTORE_PASS:?}" \
        --key-pass "pass:${KEY_PASS:-$KEYSTORE_PASS}" \
        ${KEY_ALIAS:+--ks-key-alias "$KEY_ALIAS"} --out "$OUT/colouring-pages.apk" "$OUT/aligned.apk"
else
    [ -f "$HOME/.android/debug.keystore" ] || keytool -genkeypair -keystore "$HOME/.android/debug.keystore" \
        -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
    "$BT/apksigner" sign --ks "$HOME/.android/debug.keystore" --ks-pass pass:android \
        --key-pass pass:android --out "$OUT/colouring-pages.apk" "$OUT/aligned.apk"
fi
rm -f "$OUT/colouring-pages.apk.idsig"
if [ -n "${RELEASE:-}" ]; then
    ! "$BT/aapt2" dump badging "$OUT/colouring-pages.apk" | grep -q application-debuggable || { echo "release APK is debuggable"; exit 1; }
fi
echo "built $OUT/colouring-pages.apk"
