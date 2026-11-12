# User guide

diMail's interface is in Italian only for now. This guide quotes each label as it appears in the app, with its English meaning in parentheses the first time.

Shortcuts are single keys: press <kbd>E</kbd>, not ⌘E. They work in the list and in an open thread, and they pause while you type in a text field, where only <kbd>Esc</kbd> and <kbd>⌘K</kbd> stay active. The [full table](#keyboard-shortcuts) is at the end; in the app, <kbd>?</kbd> shows it.

## The window

- **On the left**, the sidebar: **Posta in arrivo** (Inbox), **Posticipati** (Snoozed, once you have snoozed something), **Fatti** (Done), **Inviati** (Sent), **Bozze** (Drafts), **Tutta la posta** (All mail), then your **Sezioni** (Sections). <kbd>⌘\\</kbd> hides and shows it.
- **At the top**, the tabs of the inbox: **Importanti** (Important), **Altro** (Other) and any section you show as a tab. <kbd>Tab</kbd> and <kbd>⇧Tab</kbd> move between them. Next to them, back and forward (<kbd>⌘[</kbd> and <kbd>⌘]</kbd>) retrace where you have been.
- **In the middle**, the list of threads, grouped by date, or the open conversation.
- **On the right**, the [contact bar](#contact-bar).

One row of the list is always selected: move it with <kbd>J</kbd> and <kbd>K</kbd> (or the arrows), or with the mouse, since the row under the pointer becomes the selected one. Actions apply to that row. <kbd>↩</kbd> opens the thread, <kbd>Esc</kbd> goes back to the list. With a thread open, <kbd>J</kbd> and <kbd>K</kbd> go to the next and the previous thread.

To act on several threads, press <kbd>X</kbd> on each, or extend the selection with <kbd>⇧J</kbd> and <kbd>⇧K</kbd>. <kbd>Esc</kbd> clears the selection.

Closing the window hides it: diMail keeps running, keeps syncing and brings snoozed threads back on time. Click the Dock icon to show the window again; <kbd>⌘Q</kbd> quits.

## The inbox as a to-do list

In diMail the inbox holds what still needs you. A thread leaves it when you deal with it.

### Done

<kbd>E</kbd> marks the thread as **Fatto** (Done): it leaves the inbox and diMail selects the next one. On Gmail the thread is archived, so it also leaves the inbox on the web and on your phone. Done threads are in **Fatti** (<kbd>G</kbd> then <kbd>D</kbd>); <kbd>⇧E</kbd> puts one back in the inbox.

When Important is empty, diMail says **Fatto tutto** (All done) and how many threads you closed today.

### Snooze

<kbd>H</kbd> opens **Posticipa** (Snooze): the thread leaves the inbox and comes back at the top at the time you choose, marked **Promemoria** (Reminder) until you open it.

- Pick a preset with its number: **Più tardi oggi** (Later today), **Stasera** (This evening), **Domani mattina** (Tomorrow morning), **Questo fine settimana** (This weekend), **La settimana prossima** (Next week), **Tra un mese** (In a month). The list changes with the time and the day of the week.
- Or type when, in Italian: `domani alle 15` (tomorrow at 3 pm), `venerdì` (Friday), `tra 2 ore` (in 2 hours), `20/11 9:30`. A day without a time means 8 in the morning.

Things to know:

- If a reply arrives in the meantime, the thread comes back with it.
- Snoozed threads carry the Gmail label `diMail/Posticipati`, so you can find them from Gmail too. In diMail they are under **Posticipati** (<kbd>G</kbd> then <kbd>H</kbd>).
- Snoozing needs diMail to be running. If your Mac was asleep or diMail was closed at that time, the thread comes back as soon as diMail runs again.

### Trash, star, read

<kbd>#</kbd> moves the thread to the trash, <kbd>S</kbd> stars it, <kbd>U</kbd> switches between read and unread. Opening a thread marks it as read.

On the selected row, three buttons do the same with the mouse: **Fatto**, **Promemoria** and **Cestina** (Trash).

### Undo

<kbd>Z</kbd> (or <kbd>⌘Z</kbd>) undoes the last action: done, snooze, trash, star, a cleanup, a send that hasn't left yet. Press it again for the one before. After each action a short message at the bottom offers **Annulla** (Undo) too.

### Cleaning up old mail

An inbox with thousands of old threads can't work as a to-do list. **Segna come fatti i thread vecchi** (Mark old threads as done), a button above the list and a command in the palette, marks as done every thread older than a week, a month, three months, or all of them. diMail shows how many threads will leave the inbox before you confirm.

The threads are archived in Gmail too, in the background, and stay searchable. One <kbd>Z</kbd> undoes the whole cleanup. At the first launch diMail offers it once, if your inbox has a thousand or more threads older than a month.

### Offline

Without a network, the mail on your Mac stays readable and your actions wait: they go to Gmail when the connection is back. If Gmail refuses an action for good, diMail puts the thread back as it was and tells you.

diMail checks Gmail every 15 seconds while it is in front and every minute in the background. **Sincronizza ora** (Sync now), the button at the top right, checks at once.

## Important and Other

The inbox is split in two tabs. diMail decides on your Mac, with no AI and no server.

A thread goes to **Altro** (Other) when at least one of these is true:

- Gmail filed it under Promotions, Social, Updates or Forums;
- it is a mailing: it has an unsubscribe header or is marked as bulk or automatic;
- the sender is a "no-reply" address or an automatic service;
- Gmail doesn't consider it important and you have never written to that sender.

These always keep it in **Importanti** (Important), whatever the above says:

- the sender is in one of your domains;
- you have written to the sender before;
- you took part in the thread and a message is addressed to you directly, not only in Cc.

### Your domains

**I miei domini** (My domains), in Settings (<kbd>⌘,</kbd>), lists the domains whose mail always goes to Important and whose images load at once. It starts with the domain of your address, unless that is a personal mail provider such as gmail.com. Add or remove domains as you like.

### Correcting it

<kbd>M</kbd> moves the thread to the other tab. diMail then asks whether to remember it:

- **Solo questo thread** (Only this thread);
- **Sempre per** the sender (Always for this sender);
- **Sempre per tutto** the sender's domain (Always for the whole domain).

Your corrections win over every rule. To see or remove them, open the palette and run **Rivedi le correzioni di Importanti e Altro** (Review the corrections of Important and Other): <kbd>↩</kbd> removes the selected one.

## Sections

A section is a Gmail label. Your existing labels are already there, in the sidebar, and a section you create in diMail appears as a label in Gmail on the web and on your phone.

What diMail adds to a label:

- **Instructions for the AI.** In **Cosa deve finire qui** (What should end up here), describe the emails in plain words, for example "invoices, quotes and order confirmations from suppliers, also when they write from personal addresses". The AI reads the threads in your inbox, and those that arrive later, and labels the ones that match. It needs an AI key: see [AI](ai.md). Leave the field empty for a plain label.
- **A tab.** With **Mostra come scheda accanto a Importanti e Altro** (Show as a tab next to Important and Other), the section's threads in the inbox live in their own tab, and no longer show in Important or Other.
- **An order and an archive.** Drag sections to reorder them. **Archivia** (Archive) moves a section to **Archiviate** (Archived) at the bottom of the sidebar, without touching the label in Gmail.

To create one, click **+** next to **Sezioni**, or **+** after the tabs to create it as a tab. To change one, double-click it, or right-click it for **Rinomina e istruzioni AI…** (Rename and AI instructions), **Mostra come scheda in alto** (Show as a tab) and **Archivia**.

A section with instructions has a sparkle icon. When you change the instructions, the AI goes through the inbox again.

diMail has no command yet to label a thread by hand: for a section without instructions, apply the label in Gmail.

## Conversations

A thread opens as a chat, already on the last message: your messages on the right, the others on the left with the sender's initials. Each message shows its text only, without the quoted text and the signature.

Under a message, when they apply:

- **Testo citato** (Quoted text) shows what the message quotes; **Nascondi il testo citato** hides it again.
- **Firma** (Signature) shows the sender's signature.
- **Email originale** (Original email) shows the message as it was sent, with its own layout; **Torna alla conversazione** (Back to the conversation) returns. Newsletters and mailings open as the original.
- **Mostra le immagini** (Show images) loads the remote images of that message. diMail loads them by itself only for senders you have written to and for your domains; for anyone else they stay blocked, which also stops tracking pixels. Images pasted into the body of an email are not remote: they come with the message and always show.

Click the line with the sender's name and the time for the details: **Da** (From), **A** (To), **Cc**, **Data** (Date) and **Oggetto** (Subject), with full addresses you can select and copy.

**Attachments** show as cards under the message. Click a card to open the file with its app; the arrow saves a copy in your Downloads folder. An attachment is downloaded from Gmail the first time you open or save it.

Links open in your browser, never inside diMail.

At the bottom, **Rispondi** (Reply, <kbd>R</kbd>), **A tutti** (Reply all, <kbd>A</kbd>) and **Inoltra** (Forward, <kbd>F</kbd>).

## Composing

<kbd>C</kbd> starts a new email, <kbd>R</kbd> a reply, <kbd>A</kbd> a reply to all, <kbd>F</kbd> a forward. A reply opens under the conversation; a new email or a forward opens in a panel that **Ingrandisci** (Enlarge) expands over the whole mail area.

- **Recipients.** Type a name or an address in **A** (To): diMail suggests the people in your mail, the ones you write to most first. <kbd>↩</kbd>, <kbd>Tab</kbd> or a comma confirms; <kbd>⌫</kbd> on an empty field removes the last one. **Cc Ccn** adds the Cc and Bcc fields. You can paste several addresses at once.
- **Text.** Emails are written as plain text: there is no bold or italic, and pasted text loses its formatting.
- **Links.** Select some text and paste a URL with <kbd>⌘V</kbd>: the text becomes a link. <kbd>⌘Z</kbd> undoes it. It works with addresses that start with `http://`, `https://`, `mailto:` or `www.`.
- **Attachments.** Click the paper clip, or drag files from the Finder onto the window. Dropping files when no email is open starts a new one with them attached. A forward carries the attachments of the original message. Gmail's limit is 25 MB in total.
- **Signature.** Your Gmail signature, the one you set in Gmail on the web, is shown under the text and sent with the email. The **×** next to it removes it from this email; **Aggiungi la firma** (Add the signature) puts it back.
- **Quoted text.** In a reply, **Testo citato ···** shows what will be quoted under your text.
- **AI.** <kbd>⌘J</kbd> writes the reply or tidies up what you wrote: see [AI writing](#ai-writing).

<kbd>⌘↩</kbd> sends.

### Undo send

After you send, a small box at the bottom left counts down 30 seconds: **Invio tra 30 s**. Until it reaches zero:

- **Annulla** (Undo), or <kbd>Z</kbd>, stops the send and reopens the email as you left it;
- **Invia ora** (Send now), which appears when you point at the box, sends without waiting.

Then the box says **Inviata** (Sent). Emails waiting to be sent are saved: they still go if you quit diMail in the meantime, the next time it runs. If Gmail doesn't answer, diMail retries, and it never sends the same email twice.

### Closing without sending

<kbd>Esc</kbd> closes the composer. For a few seconds **Bozza chiusa** (Draft closed) offers **Riapri** (Reopen). After that the text is gone: diMail doesn't save drafts yet, so an email you are writing lives only until you send or close it.

### AI writing

With an AI key set, <kbd>⌘J</kbd> in the composer does one of two things:

- **The text is empty:** the AI writes a short reply to the last message of the thread (**Scrivi con l'AI**, Write with AI).
- **You wrote something**, even two rough notes: the AI turns it into a complete email, keeping your facts, names, numbers, dates and requests, in the language you wrote in (**Sistema con l'AI**, Tidy up with AI).

The result replaces your text, with the note **Scritto dall'AI: rileggilo prima di inviare** (Written by AI: read it before sending). **Ripristina il mio testo** (Restore my text) brings yours back, **Riscrivi** (Rewrite) asks again. Nothing is sent until you send it.

## Search

<kbd>/</kbd> puts the cursor in **Cerca nella posta** (Search mail). Results appear as you type.

- **Every word must match, by its beginning.** `prev` finds "preventivo"; `anna prev` finds the messages that have both.
- **Accents and capitals don't matter.** `lunedi` finds "lunedì".
- **It looks in** the subject, the sender, the recipients, the text of the messages (not the quoted part) and the names of the attachments.
- **Addresses.** Type an address or a piece of it: `sara.conti` finds sara.conti@acme.it, and `acme.it` finds everyone at that domain.
- **No operators yet.** Gmail's `from:`, `has:attachment` and the like are not understood.

<kbd>↩</kbd> or <kbd>↓</kbd> moves to the results; <kbd>Esc</kbd> in the field clears the search and returns to where you were.

Search covers the mail on your Mac: the whole inbox and the last 12 months, with the full text of the last 90 days and of every thread you have opened. Trash and spam are left out.

## Contact bar

The bar on the right shows who you are dealing with in the selected thread: name and address, company (from the domain), role and phone number when their signature gives them, how many emails you exchanged and since when, and your **Conversazioni recenti** (Recent conversations) with them.

<kbd>1</kbd> to <kbd>5</kbd> open the recent conversations. <kbd>I</kbd> hides and shows the bar.

Everything in it comes from your own mail. diMail asks no outside service about your contacts.

## Command palette

<kbd>⌘K</kbd> opens the palette, also while you type. Type a few letters of a command, a label or a view: the match is approximate, the letters only have to appear in that order. <kbd>↑</kbd> and <kbd>↓</kbd> choose, <kbd>↩</kbd> runs, <kbd>Esc</kbd> closes. Before you type, the commands you used last come first.

The palette lists the commands that apply where you are, each with its shortcut. Some have no key and are run from here: **Sincronizza ora**, **Nuova sezione** (New section), the themes, the review of your corrections.

<kbd>G</kbd> then <kbd>L</kbd> opens the palette on your labels only, to jump to one.

## Zoom, themes and the Dock

- **Zoom.** <kbd>⌘+</kbd> and <kbd>⌘−</kbd> make the whole interface larger or smaller, the text of the emails included, from 80% to 160%. <kbd>⌘0</kbd> goes back to normal. diMail remembers the size.
- **Themes.** The sun or moon button at the top right switches between light and dark. In the palette, **Tema: come il Mac** (Theme: like the Mac) follows the system again; **Tema: chiaro** (light) and **Tema: scuro** (dark) fix one.
- **Dock badge.** The number on diMail's Dock icon is the number of conversations in Important.

## Settings

<kbd>⌘,</kbd> opens **Impostazioni** (Settings). Every change is saved at once.

| Tab | What it holds |
|---|---|
| **AI** | Provider, API key, model, the **Prova** (Test) button and today's use. See [AI](ai.md). |
| **I miei domini** (My domains) | The domains that always go to Important. See [Your domains](#your-domains). |
| **Account Google** | The Google Cloud project and client in use, and **Sostituisci il JSON** (Replace the JSON). See [Creating your Google client](google-client.md). |

## Updates

diMail looks for a new version shortly after it starts and every 6 hours, downloads it in the background and verifies its signature. When it is ready, a box at the bottom left says **diMail 0.2.0 è pronta** (is ready), with **Riavvia** (Restart).

diMail never restarts by itself. While you have an email open or one about to leave, **Riavvia** asks you to send or close it first. If you don't restart, the new version starts the next time you open diMail. The palette also has **Riavvia diMail** (Restart diMail), which closes and reopens the app at any time.

## Keyboard shortcuts

A sequence such as <kbd>G</kbd> <kbd>I</kbd> means: press G, release it, then press I.

### Threads

| Key | Action |
|---|---|
| <kbd>E</kbd> | **Fatto** (Done) |
| <kbd>⇧E</kbd> | **Rimetti in inbox** (Move back to the inbox) |
| <kbd>H</kbd> | **Posticipa** (Snooze) |
| <kbd>#</kbd> | **Cestina** (Trash) |
| <kbd>S</kbd> | **Stella** (Star), on and off |
| <kbd>U</kbd> | **Letto / non letto** (Read or unread) |
| <kbd>M</kbd> | **Sposta in Altro** or **in Importanti** (Move to the other tab) |
| <kbd>Z</kbd> or <kbd>⌘Z</kbd> | **Annulla l'ultima azione** (Undo the last action) |

<kbd>#</kbd> is the character, wherever it is on your keyboard: <kbd>⇧3</kbd> on a US layout, <kbd>⌥à</kbd> on an Italian one.

### Writing

| Key | Action |
|---|---|
| <kbd>C</kbd> | **Scrivi un'email** (Write an email) |
| <kbd>R</kbd> | **Rispondi** (Reply) |
| <kbd>A</kbd> | **Rispondi a tutti** (Reply all) |
| <kbd>F</kbd> | **Inoltra** (Forward) |

In the composer:

| Key | Action |
|---|---|
| <kbd>⌘↩</kbd> | Send |
| <kbd>⌘J</kbd> | Write or tidy up with AI |
| <kbd>⌘V</kbd> on selected text, with a URL copied | Make a link |
| <kbd>Esc</kbd> | Shrink the enlarged panel, then close the composer |

### Moving around

| Key | Action |
|---|---|
| <kbd>J</kbd> or <kbd>↓</kbd> | **Thread successivo** (Next thread) |
| <kbd>K</kbd> or <kbd>↑</kbd> | **Thread precedente** (Previous thread) |
| <kbd>↩</kbd> | **Apri il thread** (Open the thread) |
| <kbd>Esc</kbd> | **Torna alla lista** (Back to the list); in the list, clear the selection |
| <kbd>X</kbd> | **Seleziona** (Select), in the list |
| <kbd>⇧J</kbd> or <kbd>⇧↓</kbd> | Extend the selection down |
| <kbd>⇧K</kbd> or <kbd>⇧↑</kbd> | Extend the selection up |
| <kbd>/</kbd> | **Cerca nella posta** (Search mail) |
| <kbd>⌘[</kbd> | **Indietro** (Back) |
| <kbd>⌘]</kbd> | **Avanti** (Forward) |
| <kbd>1</kbd> to <kbd>5</kbd> | Open a recent conversation from the contact bar |

### Going to a view

| Key | Action |
|---|---|
| <kbd>Tab</kbd> | **Scheda successiva** (Next tab), in the list |
| <kbd>⇧Tab</kbd> | Previous tab |
| <kbd>G</kbd> <kbd>I</kbd> | **Importanti** (Important) |
| <kbd>G</kbd> <kbd>O</kbd> | **Altro** (Other) |
| <kbd>G</kbd> <kbd>D</kbd> | **Fatti** (Done) |
| <kbd>G</kbd> <kbd>H</kbd> | **Posticipati** (Snoozed) |
| <kbd>G</kbd> <kbd>S</kbd> | **Inviati** (Sent) |
| <kbd>G</kbd> <kbd>B</kbd> | **Bozze** (Drafts) |
| <kbd>G</kbd> <kbd>A</kbd> | **Tutta la posta** (All mail) |
| <kbd>G</kbd> <kbd>L</kbd> | **Etichetta…** (Go to a label) |

### The app

| Key | Action |
|---|---|
| <kbd>⌘K</kbd> | **Comandi** (Command palette) |
| <kbd>?</kbd> | **Scorciatoie** (Shortcuts) |
| <kbd>I</kbd> | **Barra del contatto** (Contact bar) |
| <kbd>⌘\\</kbd> | **Barra a sinistra** (Sidebar) |
| <kbd>⌘,</kbd> | **Impostazioni** (Settings) |
| <kbd>⌘+</kbd>, <kbd>⌘−</kbd>, <kbd>⌘0</kbd> | Zoom in, zoom out, normal size |

### In the palette and in the snooze list

| Key | Action |
|---|---|
| <kbd>↑</kbd> <kbd>↓</kbd> | Choose |
| <kbd>↩</kbd> | Run |
| <kbd>Esc</kbd> | Close |
| <kbd>1</kbd> to <kbd>9</kbd> | In the snooze list, before you type: pick that preset |
