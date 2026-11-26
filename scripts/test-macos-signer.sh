#!/bin/sh
# Checks on a real Mac that `[updates] signer` takes a publisher's name only from
# a Developer ID. It makes a self-signed certificate with such a name in a
# keychain of its own, signs a Setup app with it and asks the update check about
# it. Then it asks about an installed app that has a real Developer ID.
#
# It adds that keychain to your search list while it runs, and takes it out
# again. The key in it is made here and thrown away.
set -eu

[ "$(uname)" = Darwin ] || { echo "This check needs a Mac."; exit 1; }
cd "$(dirname "$0")/.."

name="Developer ID Application: Example Inc. (ABCDE12345)"
work=$(mktemp -d)
keychain="$work/signer-test.keychain-db"
searched=$(security list-keychains -d user | tr '\n' ' ')

cleanup() {
  eval "security list-keychains -d user -s $searched"
  security delete-keychain "$keychain" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# A certificate anyone can make: the name, the team and even Apple's marker for
# a Developer ID leaf are whatever its maker writes.
cat > "$work/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
OU = ABCDE12345
O = Example Inc.
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
1.2.840.113635.100.6.1.13 = DER:0500
EOF
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 2 -config "$work/cert.cnf" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
/usr/bin/openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/identity.p12" -passout pass:throwaway

security create-keychain -p throwaway "$keychain"
security unlock-keychain -p throwaway "$keychain"
security import "$work/identity.p12" -k "$keychain" -P throwaway -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k throwaway "$keychain" >/dev/null
eval "security list-keychains -d user -s \"\$keychain\" $searched"

app="$work/Install Hello.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp /usr/bin/true "$app/Contents/MacOS/bach-setup"
printf payload > "$app/Contents/Resources/setup.bach"
# codesign takes a folder for an app only when it names its program.
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>bach-setup</string>
<key>CFBundleIdentifier</key><string>app.bach.signer-test</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
EOF
/usr/bin/codesign --force --timestamp=none --keychain "$keychain" --sign "$name" "$app"
/usr/bin/ditto -c -k --keepParent "$app" "$work/Hello.zip"

# Why the requirement is there: without one, codesign takes this signature.
if /usr/bin/codesign --verify --deep --strict "$app"; then
  echo "codesign --verify without a requirement ACCEPTS the self-signed app."
else
  echo "codesign --verify without a requirement refuses the self-signed app on this macOS."
fi
/usr/bin/codesign -dvv "$app" 2>&1 | grep -E '^(Authority|TeamIdentifier)=' || true

BACH_SELF_SIGNED_APP="$work/Hello.zip" \
  cargo test -p bach-pack a_self_signed_publisher_name_is_refused -- --ignored

for found in "/Applications/Firefox.app=Mozilla Corporation" "/Applications/Google Chrome.app=Google LLC" "/Applications/Visual Studio Code.app=Microsoft Corporation"; do
  # Only an intact copy says anything: one that was written into after it was
  # installed fails the strict check, whoever signed it.
  if [ -d "${found%=*}" ] && /usr/bin/codesign --verify --deep --strict "${found%=*}" 2>/dev/null; then
    echo "Checking a real Developer ID: ${found%=*}"
    BACH_DEVELOPER_ID_APP="${found%=*}" BACH_DEVELOPER_ID_NAME="${found#*=}" \
      cargo test -p bach-pack a_real_developer_id_is_accepted -- --ignored
    exit 0
  fi
done
echo "No intact app with a known Developer ID is installed, so acceptance wasn't checked."
