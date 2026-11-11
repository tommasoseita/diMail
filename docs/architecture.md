# Architecture

diMail is a [Tauri 2](https://tauri.app) app for macOS: a Rust core that syncs with Gmail and owns a local SQLite database, a thin Tauri shell that exposes the core to a web view, and a React interface that reads only from the core. There is no backend service.

```text
 ┌──────────────────────────────── diMail.app ────────────────────────────────┐
 │                                                                            │
 │  src/  React interface  ── IPC commands ──▶  src-tauri/  Tauri shell       │
 │        (web view)       ◀── events ───────         │                       │
 │                                                    ▼                       │
 │                                      crates/core/  Rust core               │
 │                                      SQLite · sync · outbox · send · AI    │
 └──────────────────────────────────────────┬─────────────────┬───────────────┘
                                            │                 │
                                            ▼                 ▼
                                     Gmail API (Google)   Mistral / OpenAI
                                     your OAuth client    your API key, optional
```

Two rules shape everything:

1. **The interface reads only from SQLite, through the core.** It never calls Gmail. A list or a thread appears as fast as a local query.
2. **Every action is written locally first, then sent to Gmail in the background.** The interface updates at once; the core retries, and rolls the local state back if Gmail refuses for good.

## Repository layout

| Path | Contents |
|---|---|
| `crates/core/` | `dimail-core`: the core, in Rust, with no dependency on Tauri |
| `src-tauri/` | The Tauri shell: IPC commands, events, plugins, window behavior, logging |
| `src/` | The interface: React and TypeScript, built with Vite |
| `scripts/` | Local signed build, the cargo runner for development, release and checks |
| `docs/` | This documentation |
| `design/` | The logo and the app icon sources |

Most code comments, the log messages and the interface strings are in Italian.

## The core: `crates/core`

The core doesn't know about Tauri, so it is tested on its own and could run elsewhere.

| Module | Role |
|---|---|
| `auth/` | OAuth 2.0 for installed apps with PKCE and a loopback redirect, the client read from Google's JSON, the token manager, the macOS Keychain |
| `gmail/` | The Gmail REST client: HTTP batches, retries, and a limiter that keeps requests inside Gmail's per-minute quota |
| `db/` | Opening SQLite (bundled, WAL mode), one writer and a few readers, schema migrations |
| `mime.rs` | From Gmail's message format to a parsed message |
| `sanitize.rs` | Cleaning HTML, HTML to plain text, text without quoted parts and signature |
| `store.rs` | Reads and writes: threads, messages, labels, the FTS5 search index, contacts, settings, your domains |
| `sync.rs` | The first sync, the download queue, the incremental sync |
| `outbox.rs` | Optimistic actions and the queue toward Gmail |
| `send.rs` | The composer's back end: building the RFC 2822 message, the send queue with undo, the Gmail signature |
| `jobs.rs` | Timed jobs: snooze |
| `bulk.rs` | The cleanup of old mail, as one resumable, undoable operation |
| `split.rs` | Important and Other: the rules and the user's corrections |
| `sections.rs` | Sections: labels with an order, an archive, a tab and AI instructions |
| `people.rs` | The contact card, from the mail alone |
| `attachments.rs` | Attachments downloaded on demand, their cache, copies to Downloads |
| `ai.rs` | Mistral or OpenAI: <kbd>⌘J</kbd> writing, sorting into sections, the daily budget |
| `account.rs` | One running account: its database, Gmail client and background tasks |
| `events.rs` | The events the core sends to the interface |
| `paths.rs` | Where the files are |

### Storage

One SQLite database per account, at `~/Library/Application Support/diMail/accounts/<address>/mail.db`. WAL mode lets the interface read while the sync writes. Search is an FTS5 index over the subject, the sender, the recipients, the unquoted text and the attachment names, with diacritics removed and prefix indexes; `store::fts_query` turns what the user types into a safe FTS5 query, so no FTS5 syntax reaches the database.

Secrets are not in the database: see [Privacy](privacy.md#what-is-in-the-keychain).

### Sync

- **First sync.** The core saves Gmail's `historyId` before listing anything, then queues what to download: the first page of the inbox, the full messages of the last 90 days, then the rest of the inbox and of the last 12 months as metadata only.
- **Download queue.** Persistent and prioritized, shared by the first sync, the incremental sync and the interface. It has an urgent lane (the user, new mail, the first page) and a background lane that leaves quota free for the urgent one. It survives a restart.
- **Incremental sync.** `history.list` from the last `historyId`: label changes are applied locally and only new messages are downloaded. It runs every 15 seconds in the foreground and every 60 in the background, and at once on wake, when the network returns and after a confirmed action.
- **Expired history.** If Gmail no longer has the `historyId`, the core does a full sync without emptying the local cache.

Opening a thread whose text isn't on the Mac yet asks for it with the highest priority: the interface shows the metadata at once and updates when the text arrives.

### Actions and the outbox

1. One transaction changes the local state and adds the operation to the outbox.
2. The interface updates immediately.
3. A worker drains the outbox toward Gmail, in batches, with retries.
4. If Gmail refuses for good, the local state goes back and the interface is told.
5. While an operation is queued, the sync doesn't overwrite the local state it changed.

Undo adds the inverse operation to the queue.

### Sending

`send.rs` builds the message as `multipart/alternative` (plain text and HTML, with the Gmail signature and the quoted text), wrapped in `multipart/mixed` when there are attachments. The message waits in a persistent send queue for the undo delay, 30 seconds. The core generates the `Message-ID` itself and, before repeating a send that failed midway, checks Sent for it: an email is never sent twice.

### AI

`ai.rs` talks to the chat completions API of Mistral or OpenAI behind a small `Llm` trait, so tests run without a network. It builds two prompts, one for writing and one for sorting threads into sections, records token counts per day and stops at the daily budget. See [AI](ai.md).

### HTML email

`sanitize.rs` cleans HTML with [ammonia](https://github.com/rust-ammonia/ammonia) before it is stored. The interface then renders it in a sandboxed `iframe` without scripts and with its own content security policy (`src/EmailFrame.tsx`). See [Privacy › How emails are displayed](privacy.md#how-emails-are-displayed).

## The shell: `src-tauri`

`src-tauri/src/lib.rs` is the whole shell.

- **Commands.** Each `#[tauri::command]` is a thin wrapper around a method of the running account: `list_threads`, `get_thread`, `act`, `undo`, `snooze`, `send_draft`, `ai_write` and so on. The interface calls them through `src/api.ts`.
- **Events.** The core publishes `CoreEvent`s on a broadcast channel; the shell forwards them to the web view as `core` events. `app_ready` tells the interface that the Keychain was read and the account resumed.
- **Start-up.** The Keychain is read off the main thread, because macOS may wait for the user to answer a prompt.
- **Plugins.** `opener` (links and files), `dialog` (file pickers), `updater` and `process` (updates and restart), and a small `external-links` plugin that stops the web view from navigating to an outside address and opens `http`, `https`, `mailto` and `tel` links in the system instead.
- **Window.** Closing the window hides it; the app keeps syncing and running timed jobs. Gaining or losing focus switches the sync between its foreground and background pace.
- **Logging.** Daily files in `~/Library/Logs/diMail/`, 7 kept. `DIMAIL_LOG` sets the filter (for example `DIMAIL_LOG=debug`). The interface reports its own errors to the same log through the `log_frontend` command.

`src-tauri/capabilities/default.json` lists what the web view may do, and `src-tauri/tauri.conf.json` holds the window, the content security policy, the bundle settings and the updater's public key and endpoint.

## The interface: `src`

| File | Role |
|---|---|
| `main.tsx`, `App.tsx` | Start-up, the first-launch screens (client JSON, sign-in) |
| `Workspace.tsx` | The working interface: sidebar, tabs, list, conversation, overlays, and the command registry |
| `commands.ts` | The `Command` type, the key router (single keys, modifiers, two-key sequences), fuzzy matching |
| `Palette.tsx` | The command palette and the shortcuts help |
| `ThreadList.tsx` | The virtualized list of threads |
| `ThreadView.tsx`, `EmailFrame.tsx` | The conversation as a chat, and the sandboxed frame for HTML email |
| `Composer.tsx`, `BodyEditor.tsx` | The composer and its text field, with links |
| `SnoozePicker.tsx`, `SectionDialog.tsx`, `BulkDialog.tsx`, `SettingsDialog.tsx`, `ContactBar.tsx` | The snooze picker, the dialogs and the contact bar |
| `updates.ts` | The update check and the restart |
| `zoom.ts`, `fileDrop.ts`, `diagnostics.ts`, `format.ts`, `icons.tsx` | Zoom, files dropped from the Finder, error reporting to the log, formatting, icons |
| `api.ts` | The typed bridge to the core's commands and events |

### The command registry

Every action is one entry in the `commands` array of `Workspace.tsx`: an id, a title, a group, its keys, where it applies and what it runs. Shortcuts, the palette and the help overlay all read from that one registry, so they can't drift apart. To add an action, add an entry there.

Keys are written as `e`, `shift+e`, `mod+k`, `#`, `arrowdown`, or `g i` for a two-key sequence. In text fields only `escape` and `mod+k` are routed.

### The mock backend

Run the interface in a browser, without Tauri, real mail or Gmail quota:

```bash
npm run dev
# then open http://localhost:1420
```

Outside Tauri, `main.tsx` loads `src/dev/mockTauri.ts`, which imitates the core's commands and events with invented data, the same at every reload. The mock isn't included in a build. Flags in the address change its behavior:

| Address | Effect |
|---|---|
| `http://localhost:1420/?primo-avvio` | The first launch: no Google client and no account |
| `http://localhost:1420/?lento` | Reading a thread takes as long as in the real app |
| `http://localhost:1420/?corpi-tardi` | The text of a message arrives after the thread opens, as for threads outside the sync window |

## Tests and checks

```bash
cargo test -p dimail-core     # the core
cargo test --workspace        # the whole workspace
npm run check                 # TypeScript type check
npm run build                 # type check and production build of the interface
```

The core's tests run without a network. `crates/core/src/sync_tests.rs` drives the sync engine and the outbox against a fake Gmail held in memory: what is done "on Gmail" shows up locally, and the other way round. The AI is tested with a fake model.

CI (`.github/workflows/ci.yml`) runs on macOS for every push to `main` and every pull request: `npm run build`, `cargo test --workspace` and `scripts/check-public.sh .`.

## Building and running

```bash
npm install
npm run tauri dev        # development build, with hot reload of the interface
scripts/build-app.sh     # target/release/bundle/macos/diMail.app
```

On macOS cargo runs binaries through `scripts/run-signed.sh` (see `.cargo/config.toml`), which signs the development binary with the same identity and identifier as the app, so the Keychain treats every build as the same app. See [CONTRIBUTING.md](../CONTRIBUTING.md#signing-and-the-keychain).

Two environment variables help while developing: `DIMAIL_DATA_DIR` moves the data folder, and `DIMAIL_LOG` sets the log filter.

Releases are described in [Releasing](releasing.md).
