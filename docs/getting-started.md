# Getting started

This guide takes you from the download to a synced inbox. The app itself takes a couple of minutes; creating your Google client, the first time, takes about ten more.

> **The interface is in Italian only for now.** This guide quotes each label as it appears in the app, with its English meaning in parentheses.

## What you need

- A Mac, Apple silicon or Intel, with a recent version of macOS.
- A Gmail or Google Workspace account.
- The JSON file of a Google OAuth client:
  - **on your own**, you create it once by following [Creating your Google client](google-client.md);
  - **in a company on Google Workspace**, an administrator creates one for everybody and sends you a setup link, or the file ([how](google-client.md#for-a-company-on-google-workspace)).
- Optional: an API key from [Mistral](https://console.mistral.ai/api-keys) or [OpenAI](https://platform.openai.com/api-keys), to turn on the AI features.

## 1. Install

1. Download `diMail.dmg` from the [latest release](https://github.com/tommasoseita/diMail/releases/latest).
2. Open it and drag **diMail** onto **Applications**.
3. Open diMail from the Applications folder. macOS asks for confirmation the first time, because the app comes from the internet: choose **Open**.

> **Run it from Applications.** A copy opened straight from the disk image can't replace itself with a newer version.

## 2. Load your Google client

The first screen says **Benvenuto in diMail** (Welcome to diMail) and asks for the client's JSON file.

- Click **Scegli il file JSON** (Choose the JSON file), or drag the file onto the window.
- With a setup link from your company, open the link instead (or paste it here with <kbd>⌘V</kbd>) and confirm with **Configura** (Set up).
- Don't have the file yet? **Come si crea il client** (How to create the client) opens the guide, in Italian. The English version is [Creating your Google client](google-client.md).

diMail stores the client in the macOS Keychain. If the file is of the wrong kind, diMail says why: the usual cause is a client of type "Web application" instead of "Desktop app".

## 3. Sign in with Google

1. Click **Accedi con Google** (Sign in with Google). Your browser opens; diMail waits, showing **Completa l'accesso nel browser…** (Complete the sign-in in the browser).
2. Choose your account and allow the access.
   - If Google shows "Google hasn't verified this app", click **Advanced**, then **Go to diMail (unsafe)**. This is expected with a client you created for a personal Gmail account: the app is yours.
   - Leave the checkbox that lets diMail read and change your mail selected.
3. The browser shows **Accesso completato** (Sign-in complete). Go back to diMail.

You have five minutes to finish the sign-in before diMail gives up; if that happens, click the button again.

> **Keychain prompt.** If macOS asks whether diMail can use the Keychain, choose **Always Allow**. diMail keeps your Google token, your client and your AI key there.

## 4. The first sync

The inbox appears within seconds and fills in while diMail downloads the rest in the background. Hover over the sync button at the top right to see how far it is: **Prima sincronizzazione** (First sync), with the number of threads on your Mac and still to download.

What diMail downloads:

- your whole inbox, and the last 12 months of the rest of your mail;
- the full text of the last 90 days. An older thread downloads its text when you open it.

You can work while it syncs. If you quit, the sync picks up where it left off.

With a thousand or more threads older than a month in the inbox, diMail offers once to start from zero: **Ripartiamo da zero?** Choose how old a thread must be and click **Segna come fatti** (Mark as done), or **Non ora** (Not now). The threads are archived in Gmail too, stay searchable, and one press of <kbd>Z</kbd> undoes it all. See [The inbox as a to-do list](user-guide.md#the-inbox-as-a-to-do-list).

## 5. Turn on AI (optional)

AI writes replies with <kbd>⌘J</kbd> and sorts emails into sections that have instructions. It uses your own key.

1. Click **Attiva l'AI** (Turn on AI) above the list, or press <kbd>⌘,</kbd> to open **Impostazioni** (Settings) on the **AI** tab.
2. **Provider:** Mistral or OpenAI.
3. **Chiave API di Mistral** or **di OpenAI** (API key): paste it and click **Salva** (Save). diMail checks the key with the provider, then stores it in the Keychain.
4. **Modello** (Model): keep the recommended one or pick another.
5. Click **Prova** (Test): diMail sends a tiny request and shows how long the answer took.

Details, and what is sent to the provider, are in [AI](ai.md).

## Your first minutes

- <kbd>J</kbd> and <kbd>K</kbd> move through the list, <kbd>↩</kbd> opens a thread, <kbd>Esc</kbd> goes back.
- <kbd>E</kbd> marks a thread as done, <kbd>H</kbd> snoozes it, <kbd>Z</kbd> undoes.
- <kbd>C</kbd> writes a new email, <kbd>R</kbd> replies.
- <kbd>⌘K</kbd> opens the command palette, <kbd>?</kbd> shows every shortcut.

## Next steps

- Learn the rest: [User guide](user-guide.md).
- See what leaves your Mac: [Privacy](privacy.md).
- Something not working? [Troubleshooting](troubleshooting.md).
