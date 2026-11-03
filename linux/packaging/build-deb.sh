#!/usr/bin/env bash
# Builds linux/target/debian/lhelper_<version>-1_amd64.deb for Ubuntu (26.04 and other recent releases).
# Needs Rust (https://rustup.rs) and a C linker (sudo apt install build-essential); installs cargo-deb if missing.
#
#   bash linux/packaging/build-deb.sh
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cargo >/dev/null || { echo "Install Rust first: https://rustup.rs" >&2; exit 1; }
command -v cc >/dev/null || { echo "Install a C toolchain first: sudo apt install build-essential" >&2; exit 1; }
cargo deb --version >/dev/null 2>&1 || cargo install cargo-deb --locked

cargo test --release
cargo deb

echo
for deb in target/debian/*.deb; do
    echo "Built $deb"
    sha256sum "$deb"
done
echo "Install: sudo apt install ./$(ls -t target/debian/*.deb | head -1)"
