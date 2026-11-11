# Releasing

This page is for maintainers who publish official builds. To build diMail for yourself, see [Build from source](../README.md#build-from-source).

Releases are built, signed and notarized on the maintainer's Mac by `scripts/release.sh`, not in CI. CI only checks and tests.

## Versions

- The version is `MAJOR.MINOR.PATCH` and lives in three files: `src-tauri/tauri.conf.json`, `Cargo.toml` (the workspace) and `package.json`.
- `scripts/bump-version.sh X.Y.Z` sets it in all three and refreshes `Cargo.lock` and `package-lock.json`. Never edit the three by hand: the release scripts stop when they disagree.
- Every release gets a section in [`CHANGELOG.md`](../CHANGELOG.md), headed `## X.Y.Z`. That section becomes the release notes.

## Project settings

[`scripts/project.env`](../scripts/project.env) is the single place for:

| Setting | Purpose |
|---|---|
| `TEAM_ID` | The Developer ID team that signs official builds |
| `NOTARY_PROFILE` | The `notarytool` keychain profile |
| `GITHUB_REPO` | The public repository that hosts the releases and the updater's `latest.json` |
| `UPDATER_KEY` | The path of the private key that signs updates |

A variable already set in the environment wins over the file.

**The bundle identifier (`io.github.tommasoseita.dimail`, in `src-tauri/tauri.conf.json`) and the signing team must never change.** macOS ties Keychain access to them: with a different identifier or team, every user would be asked again for access to their Google token and keys.

## Prerequisites

Once, on the Mac that publishes:

1. **Developer ID Application** certificate of the team, with its private key, in the login keychain.
2. **Notarization profile**, created with an App Store Connect API key:
   ```bash
   xcrun notarytool store-credentials <NOTARY_PROFILE> --key AuthKey_XXXX.p8 --key-id <id> --issuer <issuer>
   ```
3. **The updater key**, a minisign key pair:
   ```bash
   npx tauri signer generate -w ~/.tauri/dimail-updater.key
   ```
   The public key goes in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). If the private key has a password, export it as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` before releasing. **If the private key is lost, installed copies can no longer update**: keep a copy of the key and its password in a password manager.
4. **GitHub CLI**, signed in: `gh auth login`.
5. **Rust targets for both architectures**, since the build is universal:
   ```bash
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   ```

Never commit certificates, keys or tokens.

## Publishing

```bash
scripts/bump-version.sh 0.2.0
# add the "## 0.2.0" section to CHANGELOG.md
git commit -am "diMail 0.2.0"
git push origin main
scripts/release.sh "One line on what changed"
```

Run it from a clone whose `origin` is the public repository. `release.sh` refuses to start with uncommitted changes, with a commit that isn't pushed, or for a version that already has a tag or a GitHub release. It also checks first that the updater key, the Developer ID identity, the notarization profile and `gh` are in place. Then it:

1. runs the checks (`npm run check`, `cargo test --workspace`) and builds a universal app, for Apple silicon and Intel, signed with Developer ID and the hardened runtime. It verifies that the built app has the expected version and both architectures;
2. notarizes the app and staples the ticket;
3. packs `diMail.app.tar.gz`, the update archive, from the stapled app, and signs it with the updater key;
4. builds `diMail.dmg` with a link to Applications, signs it, notarizes it, staples it, and asks Gatekeeper to assess both the disk image and the app;
5. writes `latest.json` for the updater, creates a **draft** GitHub release `vX.Y.Z` on the pushed commit with the three files, checks that all three were uploaded whole, and only then publishes it as the latest release.

Everything is written to `dist-release/<version>/`, which git ignores.

The release notes are the version's section of `CHANGELOG.md`; relative links in it are rewritten to point into the repository at the release's tag. Without a section, the notes are the one line you passed to the script.

If the upload check fails, the release stays a draft and nobody receives it: look at it on GitHub, delete it and run the script again.

### Test builds

```bash
scripts/release.sh --local
```

stops after step 4: a signed, notarized DMG in `dist-release/<version>-local/`, nothing published, nothing tagged. It doesn't need a clean tree or `gh`. Use it to try a build on another Mac before publishing.

## What a release contains

| File | For |
|---|---|
| `diMail.dmg` | New installs. Always at `https://github.com/tommasoseita/diMail/releases/latest/download/diMail.dmg` |
| `diMail.app.tar.gz` | The updater: the same universal app, for both architectures |
| `latest.json` | The updater: the version, the notes, the archive's address and its signature |

## How updates work

The app uses Tauri's updater plugin (`src/updates.ts`).

- It reads `https://github.com/tommasoseita/diMail/releases/latest/download/latest.json` shortly after launch and every 6 hours. Only published releases are "latest", so a draft reaches nobody.
- It downloads the archive in the background and verifies its signature against the public key in `src-tauri/tauri.conf.json`. An archive that isn't signed with the updater key is refused.
- It then shows "diMail X.Y.Z è pronta · Riavvia" (is ready · Restart) at the bottom left. It never restarts by itself: drafts in the composer live in memory only, so while an email is open or about to leave, **Riavvia** asks the user to send or close it first.
- Development builds (`npm run tauri dev`) never check.

Two signatures are involved and both are needed: Apple's says who made the app, the updater's says that the update comes from this project.

### Why users don't see Keychain prompts after updates

Every version is signed by the same Developer ID team with the same bundle identifier, so macOS treats it as the same app and keeps giving it its Keychain items without asking.

## Before every release

- [ ] `CHANGELOG.md` has the version's section.
- [ ] `npm run check` and `cargo test --workspace` pass (the script runs them too).
- [ ] The DMG from `scripts/release.sh --local` installs and starts on another Mac or a clean user account.
- [ ] Updating from the previous version keeps the account signed in, with no Keychain prompt.

## Other scripts

| Script | Purpose |
|---|---|
| `scripts/build-app.sh` | A release build for this Mac, signed locally, in `target/release/bundle/macos/diMail.app`. Not notarized, not for distribution |
| `scripts/signing-identity.sh` | Prints the identity local builds are signed with: `DIMAIL_SIGNING_IDENTITY`, else the first Developer ID Application in the Keychain, else ad hoc |
| `scripts/run-signed.sh` | The cargo runner that signs development binaries |
| `scripts/check-public.sh <dir>` | Checks a tree before it is published: fails on a missing `LICENSE`, files that look like secrets, private documents, or text that mentions private names, ids or keys |

## CI

`.github/workflows/ci.yml` runs on macOS for every push to `main` and every pull request: it builds the interface (`npm run build`, which includes the type check), runs `cargo test --workspace` and runs `scripts/check-public.sh .` on the tree. It doesn't build, sign or publish releases.
