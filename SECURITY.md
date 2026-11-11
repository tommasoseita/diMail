# Security policy

diMail handles your mail, your Google token and your API keys. We take reports seriously and appreciate responsible disclosure.

## Reporting a vulnerability

**Please don't open a public issue.** Report it privately through GitHub: on the repository's **Security** tab, click **Report a vulnerability** ([direct link](https://github.com/tommasoseita/diMail/security/advisories/new)).

Include:

- what an attacker can do, and under which conditions;
- steps to reproduce, or a proof of concept (for a problem with how an email is displayed, the email's source helps most);
- the diMail version and the macOS version.

We'll acknowledge your report, keep you informed while we work on a fix, and credit you in the release notes unless you prefer otherwise. Please give us reasonable time to release a fix before disclosing.

## Supported versions

Security fixes go into the latest release. The app updates itself.

## In scope

- Exposure of the Google token, the OAuth client, AI keys or email content: in files, in logs, or to a host other than the ones listed in [Privacy](docs/privacy.md).
- An email that escapes its sandbox: running scripts, navigating the app's window, loading remote content from an unknown sender without the user's consent, or reaching the app's commands.
- An email or an attachment that makes diMail act on the mailbox without the user (send, delete, label).
- The sign-in flow: the loopback redirect, PKCE, the state check.
- Update integrity: installing an update that isn't signed with the project's key.

Out of scope: vulnerabilities in Gmail, in the AI providers' services or in macOS; issues that require an already compromised Mac or user account; the fact that a desktop OAuth client's `client_secret` isn't secret (see [What about the client secret?](docs/google-client.md#what-about-the-client-secret)).

## How diMail protects your data

- The Google refresh token, the OAuth client and the AI keys are stored in the macOS Keychain, tied to diMail's signature. The access token is kept in memory only.
- diMail asks Google only for `openid`, `email`, `profile` and `gmail.modify`, signs in through your browser with PKCE, and talks to Gmail directly: there is no diMail server.
- HTML email is cleaned before it is stored and displayed in a sandboxed frame without scripts, under a restrictive content security policy. Remote images are blocked for senders you don't know. Links open in your browser.
- Downloaded attachments carry macOS's quarantine mark, so Gatekeeper checks programs before they run.
- AI keys are sent only to their own provider. Logs never contain email content, tokens or keys.
- Official builds are signed with Developer ID and notarized by Apple. Updates are signed with a separate key and verified before they install.

See [Privacy](docs/privacy.md) for what data goes where.
