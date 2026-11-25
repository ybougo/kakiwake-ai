#!/usr/bin/env bash
# WSL / Linux から Windows (x86_64-pc-windows-msvc) 向けの kakiwake-ai.exe をクロスビルドする。
# sudo 不要。ツール類は ~/.cache/kakiwake-xbuild に置く。
#
#   scripts/build-windows.sh            # exe をビルド
#   scripts/build-windows.sh --copy DIR # ビルド後に DIR（例: /mnt/c/Users/me/Apps/kakiwake-ai）へコピー
#
# 初回のみ Microsoft CRT / Windows SDK をダウンロードする（xwin。Microsoft のライセンスに同意したものとみなす）。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE="${KAKIWAKE_XBUILD_DIR:-$HOME/.cache/kakiwake-xbuild}"
TARGET=x86_64-pc-windows-msvc
LLVM_VER="${LLVM_VER:-20}"
COPY_TO=""
if [[ "${1:-}" == "--copy" ]]; then COPY_TO="${2:?コピー先を指定してください}"; fi

mkdir -p "$CACHE"

# 1. Rust のターゲット
rustup target add "$TARGET" >/dev/null

# 2. llvm-rc（Windows リソースのコンパイル用）。apt からパッケージを取り出すだけで、インストールはしない
LLVM_DIR="$CACHE/llvm-$LLVM_VER"
if [[ ! -x "$LLVM_DIR/usr/lib/llvm-$LLVM_VER/bin/llvm-rc" ]]; then
  echo "==> llvm-rc を取得"
  tmp="$(mktemp -d)"
  (cd "$tmp" && apt-get download "llvm-$LLVM_VER" "libllvm$LLVM_VER" >/dev/null)
  mkdir -p "$LLVM_DIR"
  for deb in "$tmp"/*.deb; do dpkg -x "$deb" "$LLVM_DIR"; done
  rm -rf "$tmp"
fi
BIN="$CACHE/bin"
mkdir -p "$BIN"
cat >"$BIN/llvm-rc" <<EOF
#!/bin/sh
LD_LIBRARY_PATH="$LLVM_DIR/usr/lib/x86_64-linux-gnu\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}" exec "$LLVM_DIR/usr/lib/llvm-$LLVM_VER/bin/llvm-rc" "\$@"
EOF
chmod +x "$BIN/llvm-rc"

# 3. Microsoft CRT / Windows SDK
XWIN_DIR="$CACHE/xwin"
if [[ ! -d "$XWIN_DIR/crt/lib" ]]; then
  echo "==> MSVC CRT / Windows SDK を取得（xwin）"
  command -v xwin >/dev/null || cargo install xwin --locked
  xwin --accept-license --cache-dir "$CACHE/xwin-cache" splat --output "$XWIN_DIR"
  rm -rf "$CACHE/xwin-cache"
fi

# 4. フロントエンド
echo "==> フロントエンドをビルド"
(cd "$ROOT" && { [[ -d node_modules ]] || npm ci; } && npm run build)

# 5. Rust（リンカは rust-lld）
echo "==> kakiwake-ai.exe をビルド"
export PATH="$BIN:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-Lnative=$XWIN_DIR/crt/lib/x86_64 -Lnative=$XWIN_DIR/sdk/lib/um/x86_64 -Lnative=$XWIN_DIR/sdk/lib/ucrt/x86_64"
export INCLUDE="$XWIN_DIR/crt/include;$XWIN_DIR/sdk/include/ucrt;$XWIN_DIR/sdk/include/um;$XWIN_DIR/sdk/include/shared"
(cd "$ROOT" && cargo build --release -p kakiwake-ai --target "$TARGET" --features tauri/custom-protocol)

EXE="$ROOT/target/$TARGET/release/kakiwake-ai.exe"
echo "==> $EXE"
if [[ -n "$COPY_TO" ]]; then
  mkdir -p "$COPY_TO"
  cp "$EXE" "$COPY_TO/"
  echo "==> $COPY_TO/kakiwake-ai.exe にコピーしました"
fi
