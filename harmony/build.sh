#!/usr/bin/env bash
# Builds ProCraft for HarmonyOS: the app's own code (the desktop's interface, as a library) for
# the tablet, then the HAP itself.
#
#   ./build.sh            # a release build
#   ./build.sh debug      # a debug build
#
# Needs the HarmonyOS command line tools (hvigorw, ohpm, the SDK's native toolchain) and Rust with
# the `aarch64-unknown-linux-ohos` target.
set -euo pipefail

MODE="${1:-release}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
LIBS="$HERE/entry/libs/arm64-v8a"

# The toolchain: the SDK's own clang for the target, and cargo. Where the command line tools are
# is the one thing this cannot work out for itself: `DEVECO_SDK_HOME` says where the SDK is (the
# tools' `bin/` is expected beside it), and it is where the tools were installed if not.
: "${DEVECO_SDK_HOME:=/opt/harmonyos/command-line-tools/sdk}"
export PATH="$HOME/.cargo/bin:$(cd "$DEVECO_SDK_HOME/.." && pwd)/bin:$PATH"
NDK="$DEVECO_SDK_HOME/default/openharmony/native"
CC_TARGET="$NDK/llvm/bin/aarch64-unknown-linux-ohos-clang"

mkdir -p "$LIBS" "$ROOT/.cargo"
cat > "$ROOT/.cargo/config.toml" <<EOF
[target.aarch64-unknown-linux-ohos]
linker = "$CC_TARGET"
# the library's own name, as the module's other libraries are found by at run time (the linker
# would otherwise record the absolute path it was built at)
rustflags = ["-C", "link-arg=-Wl,-soname,libprocraft_ui.so"]
EOF

echo "== the app's own code (procraft_ui, $MODE) =="
if [ "$MODE" = "release" ]; then
  cargo build --release --lib --target aarch64-unknown-linux-ohos \
    --manifest-path "$ROOT/desktop/Cargo.toml"
  SOURCE="$ROOT/target/aarch64-unknown-linux-ohos/release/libprocraft_ui.so"
else
  cargo build --lib --target aarch64-unknown-linux-ohos \
    --manifest-path "$ROOT/desktop/Cargo.toml"
  SOURCE="$ROOT/target/aarch64-unknown-linux-ohos/debug/libprocraft_ui.so"
fi

# the library mustn't hold the paths it was built in
cp "$SOURCE" "$LIBS/libprocraft_ui.so"
"$NDK/llvm/bin/llvm-strip" --strip-unneeded "$LIBS/libprocraft_ui.so" 2>/dev/null || true

echo "== the HAP =="
cd "$HERE"
ohpm install --all >/dev/null 2>&1 || true
if [ "$MODE" = "release" ]; then
  hvigorw --mode module -p product=default -p buildMode=release assembleHap --no-daemon
else
  hvigorw --mode module -p product=default -p buildMode=debug assembleHap --no-daemon
fi

echo
echo "== done =="
find "$HERE/entry/build" -name '*.hap' -exec ls -lh {} + 2>/dev/null || true
