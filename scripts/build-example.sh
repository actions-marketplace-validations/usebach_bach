#!/bin/sh
# Builds the macOS example installer, dist/Hello-Setup-1.0.0.dmg, end to end:
# the release binaries (bach and the macOS setup runtime) and the setup app
# around the tiny Hello app. The macOS twin of scripts/build-example.ps1.
#
#   scripts/build-example.sh
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
cargo build --release -p bach-cli -p bach-setup
target/release/bach build examples/hello/bach.toml --platform macos --runtime target/release/bach-setup
