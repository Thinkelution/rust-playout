#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] || { echo 'This packager supports macOS Apple Silicon only.' >&2; exit 1; }
(cd web && npm ci && npm run build)
cargo build --release --locked
version=$(./target/release/rust-playout --version | cut -d ' ' -f 2)
name="rust-playout-${version}-macos-arm64"
mkdir -p "target/packages/$name"
cp target/release/rust-playout "target/packages/$name/rust-playout"
codesign --force --sign - "target/packages/$name/rust-playout"
cp LICENSE "target/packages/$name/LICENSE"
cp docs/install-macos.md "target/packages/$name/INSTALL.md"
cp Cargo.lock "target/packages/$name/Cargo.lock"
python3 scripts/license-notices.py
cp target/packages/DEPENDENCY-LICENSES.txt "target/packages/$name/DEPENDENCY-LICENSES.txt"
cp docs/third-party.md "target/packages/$name/THIRD-PARTY.md"
tar -czf "target/packages/$name.tar.gz" -C target/packages "$name"
(cd target/packages && shasum -a 256 "$name.tar.gz" > SHA256SUMS)
echo "Package: target/packages/$name.tar.gz"
