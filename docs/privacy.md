# Privacy

diMail is built so that your mail goes only where it must. There is no diMail cloud service, no account, no analytics and no telemetry. The app on your Mac talks to Gmail, and to nobody else unless you turn on a feature that needs it.

## What talks to whom

| diMail connects to | When | What goes there |
|---|---|---|
| **Google** (`accounts.google.com` in your browser, `oauth2.googleapis.com`, `gmail.googleapis.com`) | Always: it is your mailbox | The sign-in and every mail operation, through the Gmail API, with **your own** OAuth client |
| **Your AI provider** (`api.mistral.ai` or `api.openai.com`) | Only if you saved an API key, and only for the emails AI is used on | See [AI](ai.md#what-is-sent-to-the-provider) |
| **GitHub** (`github.com`) | Shortly after launch and every 6 hours | A request for the list of the latest version, then the download of the update. Nothing about you or your mail |
| **The servers of an email's images** | When an email with remote images is shown and its sender is known to you, or you click **Mostra le immagini** (Show images) | The request for the image, as any mail client makes it. The images of your own Gmail signature load when you open the composer |

That is the whole list. A link you click opens in your browser, not inside diMail.

Requests to Gmail go directly from your Mac to Google. The OAuth client that identifies the app to Google is one you created in your own Google Cloud project, or one your company created: see [Creating your Google client](google-client.md). The diMail project runs no server and can't see your mail, your tokens or how you use the app.

## What diMail asks Google for

At sign-in diMail requests these scopes and no others:

| Scope | What for |
|---|---|
| `openid`, `email`, `profile` | Your address, your name and your profile picture, to know which account this is |
| `https://www.googleapis.com/auth/gmail.modify` | Reading, sending, labeling, archiving and trashing mail, and reading your Gmail signature. This scope does not allow deleting mail for good, bypassing the trash |

The sign-in happens in your browser, on Google's pages, with PKCE. diMail never sees your Google password.

## Where your mail is stored

On your Mac, in your user's Library:

| What | Where |
|---|---|
| The mail database: messages, their text, the search index, contacts, your settings, sections and corrections, emails waiting to be sent | `~/Library/Application Support/diMail/accounts/<your address>/mail.db` (SQLite) |
| Attachments you opened or saved | `~/Library/Application Support/diMail/accounts/<your address>/attachments/` |
| The list of connected accounts | `~/Library/Application Support/diMail/app.json` |
| Logs | `~/Library/Logs/diMail/` |
| Small interface preferences: the zoom level, the commands you used last in the palette | The app's web data, in `~/Library/WebKit/io.github.tommasoseita.dimail` |

diMail downloads your whole inbox and the last 12 months of the rest of your mail, with the full text of the last 90 days; an older thread downloads its text when you open it. Attachments are downloaded only when you open or save one.

The database is a regular file, not encrypted by diMail: it is protected by your macOS user account and, if you turned it on, by FileVault. Deleting the folder deletes diMail's copy only. Your mail stays on Gmail.

## What is in the Keychain

Secrets are in the macOS Keychain, not in files. Each entry's service name starts with the app's identifier, `io.github.tommasoseita.dimail`:

| Service | Content |
|---|---|
| `….google-refresh-token` | Google's refresh token, one per account. It lets diMail get a new access token without asking you to sign in again |
| `….google-oauth-client` | The OAuth client you loaded at the first launch |
| `….ai-api-key` | Your AI keys, one per provider (`mistral`, `openai`) |

The short-lived access token is kept in memory only. macOS ties these entries to diMail's signature: another app has to ask you before it can read them.

## Logs

diMail writes a log file per day in `~/Library/Logs/diMail/` and keeps the last 7. Logs record the version, the progress of the sync, counts, timings and errors. They never contain the content of emails, tokens or keys; for AI requests they record only the number of tokens.

Read a log before attaching it to a bug report anyway: error messages can include technical details such as file paths and identifiers.

## How emails are displayed

An HTML email is untrusted content, and diMail treats it that way:

- **Cleaned before it is stored.** The core removes scripts, event handlers, comments and links with schemes other than `http`, `https`, `mailto`, `tel` and `cid`.
- **Shown in an isolated frame.** Each email is rendered in a sandboxed frame where scripts can't run, with a content security policy that allows nothing but inline styles and images.
- **Remote images blocked for unknown senders.** Images hosted on a server load by themselves only when the sender is someone you have written to, or belongs to [your domains](user-guide.md#your-domains). For anyone else they stay blocked until you click **Mostra le immagini**. This stops tracking pixels from telling a stranger that you opened their email.
- **Links leave the app.** A click on a link opens your browser (or your mail app, for `mailto:`). The email can never navigate the diMail window.
- **Attachments are quarantined.** A downloaded attachment carries the same macOS quarantine mark as a file downloaded by a browser, so Gatekeeper checks a program before it runs.

## AI

AI is off until you save a key. Then:

- <kbd>⌘J</kbd> sends the email you are writing, and the thread you are replying to, to your provider, when you press it;
- sections with instructions send the sender, the subject and the snippet of the threads in your inbox, in the background.

The exact fields are listed in [AI › What is sent to the provider](ai.md#what-is-sent-to-the-provider). Your key is sent only to its own provider. What the provider does with the requests is governed by its terms for API use.

## Updates

diMail checks `https://github.com/tommasoseita/diMail/releases/latest/download/latest.json` shortly after launch and every 6 hours. The request carries no identifier beyond what any web request does. An update is installed only if its signature matches the key built into the app, and official builds are notarized by Apple.

## Removing everything

See [Starting over](troubleshooting.md#starting-over) for how to delete diMail's data from your Mac.

## Reporting a problem

If you find a way diMail exposes more than described here, please report it privately: see [SECURITY.md](../SECURITY.md).
