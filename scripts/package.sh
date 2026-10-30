#!/usr/bin/env bash
# Builds the release package that Help > Check for Updates installs:
#   target/dist/failbrauwser-<version>-<arch>-linux.tar.gz
# holding bin/failbrauwser, lib/fb-archive, data/, install.sh (run with --prebuilt), README
# and LICENSE. The program needs GTK 3 at run time; the archive helper is self-contained.
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
NAME="failbrauwser-$VERSION"
cargo build --release --locked
make helper >/dev/null
OUT="$HERE/target/dist"
rm -rf "${OUT:?}/$NAME"
mkdir -p "$OUT/$NAME/bin" "$OUT/$NAME/lib"
install -m755 target/release/failbrauwser "$OUT/$NAME/bin/"
install -m755 target/helper/fb-archive "$OUT/$NAME/lib/"
cp -r data install.sh README.md LICENSE "$OUT/$NAME/"
tar czf "$OUT/$NAME-$ARCH-linux.tar.gz" -C "$OUT" "$NAME"
rm -rf "${OUT:?}/$NAME"
echo "$OUT/$NAME-$ARCH-linux.tar.gz"
