#!/usr/bin/env bash
# GitHub release job: temporary keychain, notarized DMG, signed Sparkle feed.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
arch=${1:?architecture required}
tag=${2:?tag required}
case "$arch:$(uname -m)" in aarch64:arm64|x86_64:x86_64) ;; *) echo 'runner architecture mismatch' >&2; exit 1 ;; esac
for name in MACOS_DEVELOPER_ID_P12_BASE64 MACOS_DEVELOPER_ID_P12_PASSWORD APPLE_API_KEY_ID APPLE_API_ISSUER_ID APPLE_API_PRIVATE_KEY_P8_BASE64 SPARKLE_ED25519_PRIVATE_KEY; do
    if [[ -z "${!name:-}" ]]; then echo "Required release secret $name is missing" >&2; exit 1; fi
done
umask 077
temporary=$(mktemp -d "${RUNNER_TEMP:-/tmp}/owt-release.XXXXXX")
keychain="$temporary/signing.keychain-db"
cleanup() {
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf "$temporary"
}
trap cleanup EXIT
printf '%s' "$MACOS_DEVELOPER_ID_P12_BASE64" | base64 -D > "$temporary/certificate.p12"
printf '%s' "$APPLE_API_PRIVATE_KEY_P8_BASE64" | base64 -D > "$temporary/AuthKey.p8"
printf '%s' "$SPARKLE_ED25519_PRIVATE_KEY" > "$temporary/sparkle-key"
password=$(openssl rand -hex 24)
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$temporary/certificate.p12" -k "$keychain" -P "$MACOS_DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain"
security default-keychain -d user -s "$keychain"
MACOS_SIGN_IDENTITY=$(security find-identity -v -p codesigning "$keychain" | sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1)
test -n "$MACOS_SIGN_IDENTITY"
export MACOS_SIGN_IDENTITY
export MACOS_NOTARY_PROFILE=OpenWalkieTalkieCI
xcrun notarytool store-credentials "$MACOS_NOTARY_PROFILE" --keychain "$keychain" --key "$temporary/AuthKey.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID"

expected=$(tr -d '\r\n' < packaging/macos/sparkle-public-key.txt)
actual=$(swift tools/sparkle-public-key.swift "$temporary/sparkle-key")
if [[ "$actual" != "$expected" ]]; then echo 'Sparkle secret does not match the embedded public key' >&2; exit 1; fi
REQUIRE_SIGNING=1 OWT_MACOS_ARCH="$arch" bash tools/package-macos.sh dist/macos
bash tools/finalize-macos-release.sh "$arch" "$tag" dist/macos "$temporary/sparkle-key"
