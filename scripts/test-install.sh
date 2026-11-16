#!/bin/sh
# Runs install.sh against a release made on the spot: a tarball of the bach
# you give it, with its SHA-256, in a folder that stands in for GitHub.
#
#   scripts/test-install.sh target/debug/bach
#
# It checks the first install, a second run, the refusals (a damaged archive,
# a wrong checksum, a missing download, a folder that isn't ours) and that the
# files end up where they should. Nothing outside a temporary folder is
# touched, and nothing is downloaded.
set -eu

bach_binary="${1:?usage: scripts/test-install.sh PATH-TO-BACH}"
here=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) echo "This test needs Linux or macOS." >&2; exit 2 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "No prebuilt bach for $(uname -m)." >&2; exit 2 ;;
esac

version=9.9.9
name="bach-$version-$os-$arch"
release="$work/release"
mkdir -p "$release" "$work/pack/$name"
cp "$bach_binary" "$work/pack/$name/bach"
chmod +x "$work/pack/$name/bach"
tar -czf "$release/$name.tar.gz" -C "$work/pack" "$name"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$release" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
else
  (cd "$release" && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
fi

failures=0
check() {
  if [ "$1" = ok ]; then echo "  ok    $2"; else echo "  FAIL  $2"; failures=$((failures + 1)); fi
}
install() { # DIR, then extra environment as NAME=VALUE
  dir="$1"; shift
  env BACH_VERSION="$version" BACH_INSTALL_FROM="${FROM:-$release}" BACH_INSTALL_DIR="$dir" HOME="$work/home" "$@" sh "$here/install.sh"
}
mkdir -p "$work/home"

echo "First install"
out=$(install "$work/bin" 2>&1) && status=0 || status=$?
[ "$status" = 0 ] && check ok "exits 0" || check no "exits 0 ($status): $out"
[ -x "$work/bin/bach" ] && check ok "bach is installed and runnable" || check no "bach is installed and runnable"
[ -f "$work/bin/.bach-cli" ] && check ok "the folder is marked as ours" || check no "the folder is marked as ours"
case "$out" in *"to add"* | *"add $work/bin to your PATH"*) check ok "it says how to put the folder on PATH" ;; *) check no "it says how to put the folder on PATH: $out" ;; esac
"$work/bin/bach" --version >/dev/null 2>&1 && check ok "the installed bach reports its version" || check no "the installed bach reports its version"

echo "A second run changes nothing"
first=$(cksum < "$work/bin/bach")
out=$(install "$work/bin" 2>&1) && status=0 || status=$?
[ "$status" = 0 ] && check ok "exits 0 again" || check no "exits 0 again ($status): $out"
[ "$first" = "$(cksum < "$work/bin/bach")" ] && check ok "the same bach is there" || check no "the same bach is there"

echo "A bach that isn't ours stays"
mkdir -p "$work/foreign"
echo "someone else's" > "$work/foreign/bach"
out=$(install "$work/foreign" 2>&1) && status=0 || status=$?
[ "$status" != 0 ] && check ok "it refuses" || check no "it refuses"
[ "$(cat "$work/foreign/bach")" = "someone else's" ] && check ok "and leaves the file alone" || check no "and leaves the file alone"
case "$out" in *"isn't from this script"*) check ok "and says why" ;; *) check no "and says why: $out" ;; esac

echo "A damaged archive"
bad="$work/bad"
mkdir -p "$bad"
cp "$release/$name.tar.gz.sha256" "$bad/"
head -c 200 "$release/$name.tar.gz" > "$bad/$name.tar.gz"
out=$(FROM="$bad" install "$work/bin2" 2>&1) && status=0 || status=$?
[ "$status" != 0 ] && check ok "it refuses" || check no "it refuses"
case "$out" in *"doesn't match its published SHA-256"*) check ok "one sentence: the checksum doesn't match" ;; *) check no "one sentence: the checksum doesn't match: $out" ;; esac
[ ! -e "$work/bin2/bach" ] && check ok "and installs nothing" || check no "and installs nothing"

echo "A checksum that fits a file that isn't an archive"
junk="$work/junk"
mkdir -p "$junk"
echo "not a tarball" > "$junk/$name.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then (cd "$junk" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"); else (cd "$junk" && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"); fi
out=$(FROM="$junk" install "$work/bin3" 2>&1) && status=0 || status=$?
[ "$status" != 0 ] && check ok "it refuses" || check no "it refuses"
case "$out" in *"isn't a readable archive"*) check ok "and says so" ;; *) check no "and says so: $out" ;; esac
[ ! -e "$work/bin3/bach" ] && check ok "and installs nothing" || check no "and installs nothing"

echo "A missing download"
out=$(FROM="$work/nowhere" install "$work/bin4" 2>&1) && status=0 || status=$?
[ "$status" != 0 ] && check ok "it refuses" || check no "it refuses"
case "$out" in *"isn't a folder"*) check ok "and says why" ;; *) check no "and says why: $out" ;; esac
mkdir -p "$work/empty"
out=$(FROM="$work/empty" install "$work/bin5" 2>&1) && status=0 || status=$?
case "$out" in *"has no $name.tar.gz"*) check ok "an empty folder is named in the message" ;; *) check no "an empty folder is named in the message: $out" ;; esac

echo "No version for a local install"
out=$(env -u BACH_VERSION BACH_INSTALL_FROM="$release" BACH_INSTALL_DIR="$work/bin6" HOME="$work/home" sh "$here/install.sh" 2>&1) && status=0 || status=$?
[ "$status" != 0 ] && check ok "it refuses" || check no "it refuses"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed."
  exit 1
fi
echo "All install.sh checks passed."
