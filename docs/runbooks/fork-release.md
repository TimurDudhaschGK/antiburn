# Fork releases

This repository is maintained at `TimurDudhaschGK/antiburn` on `fix/main`,
based on [upstream antiburn](https://github.com/antiburn/antiburn).
The bootstrap scripts download packages and checksums from this fork. The
desktop updater and manual remote-helper download link also use this fork.
The installed application directory stays the same.

Push CI runs on `fix/main`. Both release workflows require a successful push
run for the exact tagged commit on that branch. A successful `main` run or a
pull-request run does not satisfy the fork release gate. Releases remain drafts
until the assets pass the manual review in [release.md](release.md).
Use `TimurDudhaschGK/antiburn` and `fix/main` in that runbook's repository and
maintained-branch commands in place of the upstream repository and `main`.

Before the first fork release:

1. Configure the fork's own `release` environment and signing credentials.
   Forks do not inherit upstream secrets. The updater public key currently
   comes from upstream; replace it with the public half of the fork's own key
   and store the matching private half as described in
   [updater-key-recovery.md](updater-key-recovery.md). Do not publish packages
   with a public key that does not match the signing key.
2. Configure the required platform signing credentials and protected environment
   approvals described in [release.md](release.md).
3. Use a new application version and annotated tag for the fork's changed
   source. Keep published tags and assets immutable.
4. Wait for the exact commit to pass `fix/main` CI, run the release workflow,
   and review its draft before publication.
5. Verify that the fork's `latest.json`, packages, bootstrap scripts, and
   `SHA256SUMS` all refer to this fork and the intended release version.

The README install commands read scripts from `fix/main`. Those scripts require
a published fork release; they do not fall back to upstream packages when the
fork has no release. Existing upstream installations need a manual fork install
to receive the fork's updater endpoint and public key.
