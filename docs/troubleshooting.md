# Troubleshooting

diMail's messages are in Italian. This page quotes them as they appear, with their meaning. When a message isn't listed here, [the logs](#collecting-logs) usually say more.

## macOS asks for the Keychain

diMail keeps your Google token, your Google client and your AI keys in the macOS Keychain, and reads them at every launch.

- **A prompt asks whether diMail can use an item in your Keychain.** Choose **Always Allow**, so it doesn't ask again. The start screen says the same in Italian: "scegli «Consenti sempre»".
- **diMail stays on "Avvio…" (Starting).** It is waiting for your answer to that prompt, which may be behind another window. Look for it and answer.
- **You chose Deny.** diMail can't read its items and behaves as at the first launch: load the client's JSON and sign in again. The mail already downloaded stays.
- **It asks at every build.** That happens with a copy you built yourself without a stable signature: see [Signing and the Keychain](../CONTRIBUTING.md#signing-and-the-keychain). Official builds always carry the same signature, so updates don't ask.

## Loading the Google client

| Message | What to do |
|---|---|
| **il file non è un JSON…** (the file isn't JSON) | Choose the file downloaded from the Google Cloud console, named `client_secret_….json`. |
| **questo è il JSON di un client «Applicazione web»…** | The client was created as "Web application". Create another one of type **Desktop app** ([step 4](google-client.md#4-the-client)). |
| **nel JSON manca il client…** (the client is missing) | The file isn't the one of an OAuth client. Download it again from **Clients** in the console. |
| **non è il JSON di un client Google (troppo grande o non è un file)** | You picked a folder or some other, larger file. |

## Signing in with Google

- **"Google hasn't verified this app".** Expected when your client's audience is External: the app is yours and nobody asked Google to review it. Click **Advanced**, then **Go to diMail (unsafe)**.
- **"Access blocked", saying that your administrator restricted access.** Your Google Workspace blocks third-party apps. An administrator has to mark the client as **Trusted** in the Admin console: Security › Access and data control › API controls › Manage Third-Party App Access.
- **"Access blocked" for an account that isn't a test user,** or **you have to sign in again every week.** The client is External and still in "Testing": in that status only the accounts listed as test users can sign in, and the access expires after 7 days. Publish the app ([step 3.5](google-client.md#3-the-consent-screen)): the status becomes "In production".
- **non è stato concesso l'accesso a Gmail…** (access to Gmail wasn't granted). On Google's consent page the Gmail checkbox was cleared. Sign in again and leave it selected.
- **accesso negato nel browser** (access denied in the browser). The sign-in was cancelled on Google's page. Start again.
- **tempo scaduto: il login nel browser non è stato completato** (timed out). diMail waits five minutes for the sign-in. Click the button again.
- **login già in corso: completalo nel browser** (a sign-in is already in progress). Finish it in the browser tab that is already open, or wait for it to time out.
- **"Gmail API has not been used in project … or it is disabled".** Enable the Gmail API in your project ([step 2](google-client.md#2-the-gmail-api)).

## "L'accesso a Google è scaduto"

A bar at the top says **L'accesso a Google è scaduto: la posta locale resta leggibile, le azioni partono dopo il login** (Google access has expired: local mail stays readable, actions go out after you sign in). At launch, the same thing shows as a screen that says **L'accesso di … è scaduto. I dati locali sono al sicuro** (the access has expired; your local data is safe). Google no longer accepts diMail's token.

Click **Rifai il login** (Sign in again) and complete the sign-in in your browser. Nothing is lost: the mail on your Mac stays, and what you did in the meantime is sent to Gmail afterwards.

If it keeps happening every week, your client is External and still in "Testing": see above. diMail also asks you to sign in again after you replace the client with a different one in **Impostazioni** (Settings) › **Account Google**, because a token belongs to the client that obtained it.

## Mail doesn't update

The sync button at the top right shows a dot when something needs attention; point at it to read the status.

- **Offline: le azioni partono al ritorno della rete.** No connection. Your actions are queued and go to Gmail when the network is back.
- **Errore di sincronizzazione** (Sync error). Click the button to try again; if it persists, look at [the logs](#collecting-logs).
- **Gmail ha rifiutato l'azione: …** (Gmail refused the action). diMail puts the thread back as it is on Gmail and shows Gmail's reason.
- **Slow first sync, or frequent pauses.** Gmail gives each user a quota of requests per minute, and diMail stays inside it. Don't run two copies of diMail on the same account at once, for example the app and a development build: together they use the quota twice as fast.

## An email wasn't sent

- **Non inviata: … L'email è rimasta nella coda di invio** (Not sent; the email stayed in the send queue). Gmail refused the email or couldn't be reached for long. The message after "Non inviata" is the reason.
- **Gmail non risponde: riprovo** (Gmail isn't answering: retrying). diMail keeps trying by itself. It checks your Sent mail before repeating, so the email is never sent twice.
- **Gli allegati superano 25 MB…** (attachments exceed 25 MB). That is Gmail's limit for one email. Share the files with a Drive link instead.

## A link doesn't open

Links in an email open in your default browser, and `mailto:` links in your default mail app. diMail never opens a page inside its own window.

- Only `http`, `https`, `mailto` and `tel` links are opened. A link of any other kind is ignored on purpose.
- If nothing happens on a normal link, check that macOS has a default web browser set, then look in [the logs](#collecting-logs) for a line with "link esterno".

## Images don't show

diMail loads remote images only for senders you have written to and for your own domains. For anyone else, click **Mostra le immagini** (Show images) under the message. To trust a whole domain, add it in **Impostazioni** › **I miei domini** (My domains).

## AI

The messages of the AI features are explained in [AI › When something goes wrong](ai.md#when-something-goes-wrong). In short: **Prova** (Test) in **Impostazioni** › **AI** checks the key and the model and shows the provider's answer.

## Updates don't arrive

diMail checks GitHub for a new version about 15 seconds after it starts, then every 6 hours, and downloads it in the background.

- **It never restarts by itself.** Look for the box at the bottom left, **diMail … è pronta** (is ready), and click **Riavvia** (Restart). If you have an email open or about to leave, send or close it first. Otherwise the new version starts the next time you open diMail.
- **Run diMail from the Applications folder.** A copy opened from the disk image can't replace itself.
- **Check the connection** to `github.com`. A failed check is retried at the next round; the log has a line that starts with "aggiornamento".
- **Development builds don't update.** `npm run tauri dev` never checks.
- **To update by hand,** download `diMail.dmg` from the [latest release](https://github.com/tommasoseita/diMail/releases/latest) and replace the app in Applications. Your mail and your Keychain items stay.

## Collecting logs

diMail writes one log file per day and keeps the last 7:

```bash
open ~/Library/Logs/diMail/
```

The files are named `dimail.<date>.log`. They record the version, the sync, counts and errors, never the content of emails, tokens or keys.

For more detail while you reproduce a problem, quit diMail and start it from Terminal with a lower log level:

```bash
DIMAIL_LOG=debug /Applications/diMail.app/Contents/MacOS/dimail
```

Attach the file to your [bug report](https://github.com/tommasoseita/diMail/issues). Read it first: it can contain technical details such as file paths and identifiers.

## Starting over

To remove everything diMail keeps on your Mac, quit it (<kbd>⌘Q</kbd>: closing the window only hides it) and run:

```bash
rm -rf ~/Library/Application\ Support/diMail
rm -rf ~/Library/Logs/diMail
rm -rf ~/Library/WebKit/io.github.tommasoseita.dimail
```

Then open **Keychain Access**, search for `io.github.tommasoseita.dimail` and delete the entries: the Google token, the Google client and the AI keys.

The next launch is a first launch. Before you do this, know that:

- your mail on Gmail isn't touched, and neither are the labels diMail created there;
- an email still waiting in the send queue is lost;
- threads you snoozed stay out of the inbox, under the Gmail label `diMail/Posticipati`, and no longer come back by themselves: move them back to the inbox from Gmail.
