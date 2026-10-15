#!/usr/bin/env bash
# Finish an already notarized package, including preserved CI packages.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
arch=${1:?architecture required}
tag=${2:?tag required}
directory=$(cd "${3:?package directory required}" && pwd)
key=${4:?Sparkle key file required}
case "$arch" in aarch64|x86_64) ;; *) exit 2 ;; esac
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 2
version=${tag#v}
dmg="OpenWalkieTalkie-$version-macos-$arch.dmg"
manifest="$directory/release-manifest.json"
if [[ ! -f "$manifest" ]]; then manifest="$directory/package-manifest-$arch.json"; fi
jq -e --arg version "$version" --arg arch "$arch" --arg dmg "$dmg" \
    '.version == $version and .arch == $arch and .dmg.name == $dmg and
     .distributable == true and .notarized == true and .bundle.signature == "developer-id"' "$manifest" >/dev/null
test "$(shasum -a 256 "$directory/$dmg" | awk '{print $1}')" = "$(jq -r '.dmg.sha256' "$manifest")"
test "$(stat -f %z "$directory/$dmg")" = "$(jq -r '.dmg.bytes' "$manifest")"
expected=$(tr -d '\r\n' < "$root/packaging/macos/sparkle-public-key.txt")
test "$(swift "$root/tools/sparkle-public-key.swift" "$key")" = "$expected"

# Apple's final ticket check can disconnect even after a successful staple.
bash "$root/tools/staple-macos.sh" "$directory/$dmg" --validate-only
codesign --verify --strict "$directory/$dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$directory/$dmg"
umask 077
temporary=$(mktemp -d "${RUNNER_TEMP:-/tmp}/owt-feed.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
mkdir "$temporary/updates"
cp "$directory/$dmg" "$temporary/updates/$dmg"
sparkle="$root/target/sparkle/2.9.6"
feed="$directory/appcast-$arch.xml"
"$sparkle/bin/generate_appcast" --ed-key-file "$key" \
    --download-url-prefix "https://github.com/$GITHUB_REPOSITORY/releases/download/$tag/" \
    --link "https://github.com/$GITHUB_REPOSITORY/releases/tag/$tag" \
    --maximum-deltas 0 -o "$feed" "$temporary/updates"
xmllint --noout "$feed"
signature=$(sed -n 's/.*sparkle:edSignature="\([^"]*\)".*/\1/p' "$feed" | head -1)
length=$(sed -n 's/.*length="\([0-9]*\)\".*/\1/p' "$feed" | head -1)
test "$length" = "$(stat -f %z "$directory/$dmg")"
grep -F "<sparkle:version>$version</sparkle:version>" "$feed"
grep -F "https://github.com/$GITHUB_REPOSITORY/releases/download/$tag/$dmg" "$feed"
"$sparkle/bin/sign_update" --verify --ed-key-file "$key" "$directory/$dmg" "$signature"
"$sparkle/bin/sign_update" --verify --ed-key-file "$key" "$feed"
jq --arg sha "$(shasum -a 256 "$directory/$dmg" | awk '{print $1}')" \
   --argjson bytes "$(stat -f %z "$directory/$dmg")" \
   '.sparkle.appcast_signed = true | .dmg.sha256 = $sha | .dmg.bytes = $bytes' \
   "$manifest" > "$temporary/package-manifest.json"
mv "$temporary/package-manifest.json" "$directory/package-manifest-$arch.json"
