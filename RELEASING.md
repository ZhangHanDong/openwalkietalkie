# Builds, releases, and updates

OpenWalkieTalkie uses the same release model as
[OpenMicroKbd](https://github.com/conol-ai/openmicrokbd): a GitHub Release
contains the Mac app, signed Sparkle feeds, firmware, a release manifest,
checksums, and artifact attestations.

## CI

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`, and
can be started manually. It tests the shared firmware and renderer for
LilyGo, Rev A, and Rev B; builds all three firmware configurations; tests
the Mac host and app; and compiles the Sparkle bridge. Rev A and Rev B
application images and debug ELFs are kept as workflow artifacts.

## Configure signed publishing

The GitHub environment **release** needs these secrets. Names match
OpenMicroKbd so existing Apple credentials can be provisioned the same way.

| Secret | Contents |
| --- | --- |
| `MACOS_DEVELOPER_ID_P12_BASE64` | Base64 Developer ID Application certificate and private key, exported as PKCS#12 |
| `MACOS_DEVELOPER_ID_P12_PASSWORD` | Nonempty password for that PKCS#12 export |
| `APPLE_API_KEY_ID` | App Store Connect key ID |
| `APPLE_API_ISSUER_ID` | App Store Connect issuer ID |
| `APPLE_API_PRIVATE_KEY_P8_BASE64` | Base64 private `.p8` key |
| `SPARKLE_ED25519_PRIVATE_KEY` | Base64 32-byte seed matching `packaging/macos/sparkle-public-key.txt` |

The OpenWalkieTalkie Sparkle key is stored locally under the Keychain
account `conol-ai.openmicrodial`. Keep a secure backup; changing the public
key alone prevents installed apps from accepting future updates. The
workflow derives the public key from its protected secret and verifies it
against the committed key before publishing. Never commit private keys.

Missing credentials, a mismatched Sparkle key, failed code signing, failed
notarization, or a failed test stops the release. An ad-hoc developer build
cannot become a published automatic update. Credentials are imported into
a temporary runner keychain and removed when the packaging step exits.

Configure environment reviewers and tag protection to match who may ship
releases. The `release` environment itself does not add reviewers by default.

## Ship a release

1. Bump `host/Cargo.toml`'s workspace version for the app and daemon.
   Bump the root `Cargo.toml` firmware version whenever firmware bytes or
   behavior change. Refresh the corresponding lockfiles. Host and firmware
   versions are independent, but both must be stable `MAJOR.MINOR.PATCH`.
2. Commit to `main` and wait for CI. Exercise the exact firmware on hardware,
   including interrupted USB transfer followed by recovery. CI cannot test
   the microphone, speaker, physical controls, or power loss.
3. Create and push `v<host-version>`. For host version `0.2.0`, use
   `git tag v0.2.0` followed by `git push origin v0.2.0`.

The release workflow verifies that the tagged commit belongs to `main`,
reruns CI, builds native Apple Silicon and Intel packages, signs and
notarizes the DMGs, signs and verifies both Sparkle feeds, creates the
manifest and SHA-256 checksums, and attests the assets. It uploads a draft
before making the complete release public. Published versions are immutable;
use a new tag to change a published release.

Windows packaging is not included: this project's companion app and voice
engines currently target macOS 13 or later.

## App updates

The packaged app links Sparkle 2.9.6. Developer ID builds enable automatic
checks on launch and every six hours and default to automatic downloads and
installation using Sparkle's standard UI and preferences. Both the feed and
update archive must be signed. **Check for updates** in the tray or
**Service → Updates** starts a manual check.

The app registers its bundled background agent on launch, separately from
the optional GUI login item. After an update, it compares the running
daemon's version and recorded launch path against its bundled daemon. A mismatch
requests `service.restart` with `when_idle:true`; active PTT, pending final
recognition, speech playback, and firmware operations defer the restart.
The daemon exits with code 75 and launchd runs the new bundled executable.
An unrelated development daemon is left alone.

Source and ad-hoc builds keep Sparkle installation disabled. They still
support firmware checks and updates through the running daemon.

## Device firmware updates

With one deck connected by **USB**, open **Service → Updates → Check
firmware**. A newer release also produces a notification after the automatic
check. Review the version, click **Update firmware…**, then **Install
firmware**. The view shows download, installation, reconnection, and errors.

The daemon downloads the stable manifest from this repository's GitHub
Release. It validates product, board, protocol, flash layout, asset URL,
size, SHA-256, board marker, and vector table. A matching image bundled
inside the app is used when available; otherwise the release asset is
downloaded. The selected serial number is checked again before recovery and
used throughout reconnection. Multiple USB decks are refused during discovery.

Normal HID traffic pauses during installation and resumes on success or
failure. Transfers use the existing OWTB protocol with sector CRCs; the
bootloader commits the vector sector last. Only the application partition
is written. The updater never writes the bootloader, flash table, or pairing
journal and never updates over Bluetooth.

If the deck is already in recovery or its older firmware cannot identify its
board, choose the physical revision printed on the board. Recovery mode
does not report its board, so this choice cannot be detected automatically.
After a cable interruption, reconnect, check again, and retry from recovery.
Success requires the same serial number to report the expected board and
version after reboot.

Daemon API calls are `firmware.status` (`observe`), `firmware.check`
(`admin`, optional `force`), and `firmware.install` (`admin`, with `board`,
`serial`, and `version` copied from the checked release). Installation is
always explicit; scheduled checks never flash a device.

## Local validation

```sh
python3 -m unittest discover -s tools/tests -p 'test_release.py'
bash tools/build-firmware-release.sh hw-a dist/firmware
bash tools/build-firmware-release.sh hw-b dist/firmware
tools/package-macos.sh dist/macos
```

The last command produces an ad-hoc package unless signing credentials are
provided. `REQUIRE_SIGNING=1` requires both code signing and notarization.
`release-manifest.json` generated by packaging describes that package;
the release workflow combines all verified packages and firmware into the
public updater manifest using `tools/release.py manifest`.
