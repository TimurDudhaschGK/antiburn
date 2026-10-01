# Fork releases

This repository is maintained at `TimurDudhaschGK/antiburn` on `fix/main`,
based on [upstream antiburn](https://github.com/antiburn/antiburn).
The bootstrap scripts download packages and checksums from this fork. The
desktop updater and manual remote-helper download link also use this fork.
The installed application directory stays the same.

The fork publishes Windows x64/ARM64 and Linux x64/ARM64 packages. macOS source
support remains in the repository, but this fork does not publish macOS packages
without its own Apple Developer signing and notarization credentials. Windows
installers are unsigned; SHA-256 checksums and the fork's updater signatures
remain required.

Push CI runs on `fix/main`. Both release workflows require a successful push
run for the exact tagged commit on that branch. A successful `main` run or a
pull-request run does not satisfy the fork release gate. Releases remain drafts
until the assets pass the manual review in [release.md](release.md).
Use `TimurDudhaschGK/antiburn` and `fix/main` in that runbook's repository and
maintained-branch commands in place of the upstream repository and `main`.

The fork's `release` environment accepts only `antiburn-v*` tags. Its own
password-protected updater key was generated on 1 October 2026, and the matching
private key and password are configured as environment secrets. The public half
is committed in `tauri.conf.json`. The local recovery files are stored outside
the repository with access restricted to the current Windows user. Move them
into the maintainer's password manager or offline custody before removing the
local backup. `ALLOW_UNSIGNED_WINDOWS=true` declares the current Windows signing
mode.

For each fork release:

1. Preserve the fork's updater key and the tag restriction on the `release`
   environment. Follow [updater-key-recovery.md](updater-key-recovery.md) for
   custody or rotation. Forks do not inherit upstream secrets.
2. Keep the four-target release matrix, artifact counts, and updater platform
   inventory aligned. Add macOS only with Apple signing credentials and matching
   bootstrap and manifest validation. Add Authenticode credentials when available
   and remove `ALLOW_UNSIGNED_WINDOWS` at that time.
3. Use a new application version and annotated tag for the fork's changed
   source. Keep published tags and assets immutable.
4. Wait for the exact commit to pass `fix/main` CI, run the release workflow,
   and review its draft before publication.
5. Verify that the fork's `latest.json`, packages, bootstrap scripts, and
   `SHA256SUMS` all refer to this fork and the intended release version.

The README install commands download the scripts attached to the latest fork
release. Its `blob/fix/main` links show the maintained source for inspection.
The scripts do not fall back to upstream packages. Existing upstream
installations need a manual fork install to receive the fork's updater endpoint
and public key.

Antigravity 2.0 IDE/`agy` credential refresh needs both optional Google installed-app
OAuth inputs. Configure both `GOOGLE_ANTIGRAVITY_2_IDE_AGY_OAUTH_CLIENT_ID` and
`GOOGLE_ANTIGRAVITY_2_IDE_AGY_OAUTH_CLIENT_SECRET` together in the release
environment to include refresh support. Without them, local session analysis
remains available; this fork release does not promise credential refresh.

Linux packaging pins the linuxdeploy [1-alpha-20251107-1 release](https://github.com/linuxdeploy/linuxdeploy/releases/tag/1-alpha-20251107-1) by dated release URL and SHA-256 for each architecture. Keep those pins on a dated release. Its continuous release replaces assets and can invalidate older IDs.
