#!/bin/sh
# A real install, update, repair and removal of the example app on this Mac,
# the macOS twin of scripts/e2e.ps1. It runs with a throwaway HOME, so nothing
# of the person running it is touched: the app, its links, its launch agents and
# its records all live in a temporary folder that is deleted at the end.
#
#   scripts/e2e-macos.sh [debug|release]     (needs `cargo build` first)
set -eu

profile="${1:-debug}"
root="$(cd "$(dirname "$0")/.." && pwd)"
bach="$root/target/$profile/bach"
runtime="$root/target/$profile/bach-setup"
[ -x "$bach" ] && [ -x "$runtime" ] || { echo "build first: cargo build ${profile#debug}"; exit 1; }

work="$(mktemp -d "${TMPDIR:-/tmp}/bach-e2e.XXXXXX")"
export HOME="$work/home"
# Launch agents are written but never loaded into this Mac's launchd.
export BACH_NO_LAUNCHCTL=1
mkdir -p "$HOME/Desktop" "$HOME/Applications" "$HOME/Library"
mounted=""
cleanup() {
  [ -z "$mounted" ] || hdiutil detach -quiet -force "$mounted" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

ok() { echo "ok    $1"; }
fail() { echo "FAIL  $1"; exit 1; }
check() { if eval "$2"; then ok "$1"; else fail "$1"; fi; }

# The example, copied so a second version can differ from the first.
cp -R "$root/examples/hello" "$work/hello"
cfg="$work/hello/bach.toml"
sed -i.bak -e 's|../../dist/Hello-Setup-{version}.dmg|dist/Hello-Setup-{version}.dmg|' -e 's|^\[installer\]|[installer]\nstart_with_windows = false\ndesktop_shortcut = false|' "$cfg"

printf '\n[[macos.agents]]\nname = "sync"\nfile = "Hello.app/Contents/MacOS/hello"\nrun_at_load = false\ninterval = 3600\n' >> "$cfg"

build() {
  "$bach" build "$cfg" --platform macos --runtime "$runtime" --app-version "$1" --fast >/dev/null
}

# Opens a built disk image and copies its Setup app out, so it runs after the image is gone.
unpack() {
  dest="$work/run-$1"
  mkdir -p "$dest" "$work/mnt"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$work/mnt" "$work/hello/dist/Hello-Setup-$1.dmg"
  mounted="$work/mnt"
  ditto "$work/mnt/Install Hello.app" "$dest/Install Hello.app"
  hdiutil detach -quiet "$work/mnt"
  mounted=""
  setup="$dest/Install Hello.app/Contents/MacOS/bach-setup"
}

# The app is one bundle, so it goes straight into Applications (no folder of its own), and its
# uninstaller lives out of sight in the Library: the app is the only thing to see, and the Trash is the uninstall.
dir="$HOME/Applications"
ubundle="$HOME/Library/Application Support/Bach/uninstallers/Hello/Uninstall Hello.app"
uninstaller="$ubundle/Contents/MacOS/bach-setup"

echo "== a first install"
build 1.0.0
unpack 1.0.0
"$bach" verify "$work/hello/dist/Hello-Setup-1.0.0.dmg" >/dev/null && ok "bach verify accepts the disk image"
"$setup" --silent --with desktop-shortcut,autostart
check "the app is installed" "[ -x '$dir/Hello.app/Contents/MacOS/hello' ]"
check "the program keeps its executable permission" "'$dir/Hello.app/Contents/MacOS/hello' | grep -q 'Hello from Bach'"
check "the uninstaller is out of sight in the Library" "[ -x '$uninstaller' ] && [ -f '$ubundle/Contents/Info.plist' ]"
check "the app sits directly in Applications, in no folder of its own" "[ -d '$dir/Hello.app' ] && [ ! -e '$dir/Hello' ]"
check "nothing but the app is in Applications" "[ \"\$(ls '$dir')\" = 'Hello.app' ]"
check "the Desktop link is there" "[ -L '$HOME/Desktop/Hello.app' ]"
check "open at login is a launch agent" "grep -q 'RunAtLoad' '$HOME/Library/LaunchAgents/app.bach.login.Hello.plist'"
check "the app's background agent is a launch agent" "grep -q 'StartInterval' '$HOME/Library/LaunchAgents/app.bach.agent.Hello.sync.plist'"
check "the install is recorded" "grep -q '\"version\": \"1.0.0\"' '$HOME/Library/Application Support/Bach/installs/Hello.json'"
check "the uninstaller reads as a setup file" "'$bach' inspect '$ubundle' | grep -q 'Uninstaller for Hello 1.0.0'"
check "no half-installed leftovers" "[ -z \"\$(find '$dir' -name '*.bach-*' -o -name '.bach-update')\" ]"

echo "== a repair"
rm "$dir/Hello.app/Contents/Info.plist"
"$setup" --silent --repair
check "repair writes back the missing file" "[ -f '$dir/Hello.app/Contents/Info.plist' ]"

echo "== an update that drops a file and adds one"
rm "$work/hello/app-macos/Hello.app/Contents/Info.plist"
echo "new in 2" > "$work/hello/app-macos/Hello.app/Contents/NEW"
printf '#!/bin/sh\necho "Hello 2"\n' > "$work/hello/app-macos/Hello.app/Contents/MacOS/hello"
build 2.0.0
unpack 2.0.0
"$setup" --silent
check "the new version runs" "'$dir/Hello.app/Contents/MacOS/hello' | grep -q 'Hello 2'"
check "the added file arrived" "[ -f '$dir/Hello.app/Contents/NEW' ]"
check "the dropped file is gone" "[ ! -e '$dir/Hello.app/Contents/Info.plist' ]"
check "the record says 2.0.0" "grep -q '\"version\": \"2.0.0\"' '$HOME/Library/Application Support/Bach/installs/Hello.json'"
check "the uninstaller is the new one" "'$bach' inspect '$ubundle' | grep -q 'Uninstaller for Hello 2.0.0'"
check "the choices from the first install were kept" "[ -L '$HOME/Desktop/Hello.app' ] && [ -f '$HOME/Library/LaunchAgents/app.bach.login.Hello.plist' ]"

echo "== a downgrade is refused"
unpack 1.0.0
if "$setup" --silent; then fail "an older version replaced a newer one"; else ok "the downgrade is refused"; fi
check "2.0.0 is still installed" "'$dir/Hello.app/Contents/MacOS/hello' | grep -q 'Hello 2'"

echo "== a damaged setup file"
cp -R "$work/run-2.0.0" "$work/damaged"
payload="$work/damaged/Install Hello.app/Contents/Resources/setup.bach"
size=$(wc -c < "$payload")
head -c $((size - 200)) "$payload" > "$payload.cut" && mv "$payload.cut" "$payload"
if "$work/damaged/Install Hello.app/Contents/MacOS/bach-setup" --silent --dir "$work/never"; then fail "a damaged setup file installed"; else ok "a damaged setup file is refused"; fi
check "nothing was written for it" "[ ! -e '$work/never' ]"

echo "== data and removal"
mkdir -p "$HOME/Library/Application Support/Hello" && echo "user data" > "$HOME/Library/Application Support/Hello/settings.txt"
echo "mine" > "$dir/my-own-file.txt"
"$uninstaller" --silent
check "the app is gone" "[ ! -e '$dir/Hello.app' ] && [ ! -e '$ubundle' ]"
check "the links are gone" "[ ! -e '$HOME/Applications/Hello.app' ] && [ ! -L '$HOME/Desktop/Hello.app' ]"
check "the background agent is gone" "[ ! -e '$HOME/Library/LaunchAgents/app.bach.agent.Hello.sync.plist' ]"
check "the launch agent and the record are gone" "[ ! -e '$HOME/Library/LaunchAgents/app.bach.login.Hello.plist' ] && [ ! -e '$HOME/Library/Application Support/Bach/installs/Hello.json' ]"
check "the update task is gone" "[ ! -e '$HOME/Library/LaunchAgents/app.bach.update.Hello.plist' ]" || true
check "the user's data stays unless asked" "[ -f '$HOME/Library/Application Support/Hello/settings.txt' ]"
check "a file of the user's own in the folder stays" "[ -f '$dir/my-own-file.txt' ]"

echo "== a second removal with the data"
rm -rf "$dir/Hello.app"
build 2.0.0
unpack 2.0.0
"$setup" --silent
"$uninstaller" --silent --delete-data
check "the data folder went when asked" "[ ! -e '$HOME/Library/Application Support/Hello' ]"
check "Applications itself stays" "[ -d '$dir' ]"

echo "== the app is moved to the Trash"
build 2.0.0
unpack 2.0.0
"$setup" --silent --with desktop-shortcut,autostart
check "the watcher is installed" "[ -f '$HOME/Library/LaunchAgents/app.bach.guard.Hello.plist' ]"
"$uninstaller" --cleanup-if-removed
check "nothing is cleaned up while the app is there" "[ -x '$dir/Hello.app/Contents/MacOS/hello' ] && [ -f '$HOME/Library/Application Support/Bach/installs/Hello.json' ]"
mkdir -p "$HOME/Library/Application Support/Hello" && echo "user data" > "$HOME/Library/Application Support/Hello/settings.txt"
mv "$dir/Hello.app" "$work/Trash-Hello.app"
"$uninstaller" --cleanup-if-removed
check "its links, agents and record are cleaned up" "[ ! -L '$HOME/Desktop/Hello.app' ] && [ ! -e '$HOME/Library/LaunchAgents/app.bach.login.Hello.plist' ] && [ ! -e '$HOME/Library/LaunchAgents/app.bach.guard.Hello.plist' ] && [ ! -e '$HOME/Library/Application Support/Bach/installs/Hello.json' ]"
check "the uninstaller is gone too" "[ ! -e '$ubundle' ]"
check "the user's data stays" "[ -f '$HOME/Library/Application Support/Hello/settings.txt' ]"

echo "all checks passed"
