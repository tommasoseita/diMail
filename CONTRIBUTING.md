# Contributing to diMail

Thank you for helping. diMail is a small project that cares about speed, privacy and the keyboard; this guide explains how to work on it so your change lands quickly.

## Before you start

- **Bugs:** search the [issues](https://github.com/tommasoseita/diMail/issues) first, then open one. Say what you did, what you expected and what happened, with the diMail version and the relevant lines of the log (see [Collecting logs](docs/troubleshooting.md#collecting-logs)).
- **Features:** open an issue to discuss the idea before writing code. diMail deliberately does few things; a short conversation saves you a pull request that can't be merged.
- **Security issues:** never in public issues. See [SECURITY.md](SECURITY.md).

## Setting up

You need:

- macOS;
- Apple's Command Line Tools (`xcode-select --install`);
- Rust, installed with [rustup](https://rustup.rs);
- Node 22.

```bash
git clone https://github.com/tommasoseita/diMail.git
cd diMail
npm install
```

[Architecture](docs/architecture.md) explains how the core, the shell and the interface fit together.

### The interface, with the mock backend

Most interface work needs neither a Google account nor Tauri:

```bash
npm run dev
```

Open http://localhost:1420 in a browser. Outside Tauri the interface runs on `src/dev/mockTauri.ts`, a mock backend with invented data: no real mail, no Gmail quota. Add `?primo-avvio` to the address for the first-launch screens, `?lento` for realistic delays, `?corpi-tardi` for messages whose text arrives late. The mock is not part of a build.

### The app

```bash
npm run tauri dev        # the app, in development
scripts/build-app.sh     # target/release/bundle/macos/diMail.app
```

To run against real mail you need your own Google client, like any user: see [Creating your Google client](docs/google-client.md).

A development build uses the same data folder and the same Keychain entries as the installed app. Two things follow:

- **Don't run `tauri dev` and diMail.app on the same account at once.** Two syncs use Gmail's quota twice as fast, and they share one database.
- Set `DIMAIL_DATA_DIR` to another folder to keep a development database apart from your real one.

### Signing and the Keychain

diMail keeps its secrets in the macOS Keychain, and macOS ties Keychain access to the app's signature. With an ad hoc signature, every build looks like a new app and macOS asks again for access.

`scripts/signing-identity.sh` chooses the identity for local builds:

1. `DIMAIL_SIGNING_IDENTITY`, if you set it to the name of a code signing identity in your Keychain;
2. otherwise the first "Developer ID Application" identity it finds;
3. otherwise an ad hoc signature.

`scripts/build-app.sh` signs the app with it. Development builds are signed too: on macOS cargo starts binaries through `scripts/run-signed.sh` (see `.cargo/config.toml`), which signs the `dimail` binary with that identity and the app's identifier before running it. With a stable identity the Keychain treats every build as the same app.

## Making a change

- **One topic per pull request**, with a description of what changes for the user and how you tested it.
- **Tests.** Add or update tests for logic in `crates/core`. The sync engine and the outbox are tested against a fake Gmail in `crates/core/src/sync_tests.rs`; the AI against a fake model.
- **Privacy.** Never log, store or send email content, tokens or keys beyond what [Privacy](docs/privacy.md) describes. If your change affects what leaves the Mac, update that page.
- **Gmail quota.** Every Gmail call has a cost in quota units (`crates/core/src/gmail/mod.rs`). Don't add calls on the path of a user action without counting them.
- **The interface reads only from the core.** It never calls Gmail, and an action is written locally before it goes to Gmail. Keep it that way.
- **Commands.** A new action is an entry in the command registry in `src/Workspace.tsx`, so that the shortcut, the palette and the help overlay stay in step.
- **Strings.** The interface is in Italian only for now: write new labels in Italian, in the tone of the existing ones.
- **Docs.** If you change behavior, update the matching page in [`docs/`](docs), including the shortcut table in the [user guide](docs/user-guide.md#keyboard-shortcuts). UI changes are easier to review with a screenshot.

### Style

- Rust: edition 2024. Keep `crates/core` free of Tauri, so it stays testable on its own.
- TypeScript: strict mode, no unused locals or parameters. Discuss a new runtime dependency in the issue first.
- Comments explain *why*, not what. Most existing comments are in Italian; English is welcome in new code. Docs are in English.
- Commit messages: a short summary line that says what changes, then details if needed.

## Checks

Every pull request runs, on macOS:

- the type check and the production build of the interface;
- the Rust tests of the whole workspace;
- `scripts/check-public.sh`, which fails if the tree contains something that looks like a secret.

Run them locally before pushing:

```bash
npm run check
cargo test --workspace
scripts/check-public.sh .
```

## Releases

Maintainers publish releases from their own Mac: see [Releasing](docs/releasing.md).

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE) of the project.

Please follow our [Code of Conduct](CODE_OF_CONDUCT.md).
