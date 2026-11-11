# AI

AI in diMail is optional. Without a key the app works in full, minus the two features below. With a key, requests go straight from your Mac to the provider you chose: there is no diMail server in between and you pay the provider directly for what you use.

The app's labels are in Italian; the English meaning is in parentheses.

## What AI is used for

Two things, and nothing else:

| Feature | When it runs | Where |
|---|---|---|
| **Writing with <kbd>⌘J</kbd>** | Only when you press <kbd>⌘J</kbd> or click the button in the composer | [User guide › AI writing](user-guide.md#ai-writing) |
| **Sorting into sections** | In the background, for the threads in your inbox, when at least one section has instructions | [User guide › Sections](user-guide.md#sections) |

The split between Important and Other does not use AI: it is computed on your Mac from rules. Search doesn't use AI either.

## Turning it on

Press <kbd>⌘,</kbd> to open **Impostazioni** (Settings) on the **AI** tab, or click **Attiva l'AI** (Turn on AI) above the list.

1. **Provider:** **Mistral** or **OpenAI**.
2. **Chiave API** (API key): create one in the provider's console ([Mistral](https://console.mistral.ai/api-keys), [OpenAI](https://platform.openai.com/api-keys)), paste it and click **Salva** (Save). diMail first checks the key with the provider and refuses one that the provider doesn't accept.
3. **Modello** (Model): see below.
4. **Prova** (Test) sends a minimal request to the chosen model and shows how long it took, or the provider's error.

You can save a key for each provider and switch between them: the provider button shows **chiave ✓** when its key is saved. **Togli la chiave** (Remove the key) deletes it.

## Your key

The key is stored in the macOS Keychain, under the service `io.github.tommasoseita.dimail.ai-api-key`, one entry per provider. It is never written to a file or to the logs, and it is sent only to its own provider.

## Models

The list offers a few models for each provider, with one marked **consigliato** (recommended):

| Provider | Models in the list | Default |
|---|---|---|
| Mistral | `mistral-small-2603`, `mistral-medium-2604`, `mistral-large-4-0` | `mistral-small-2603` |
| OpenAI | `gpt-5.4-mini`, `gpt-6-luna`, `gpt-6.1-terra`, `gpt-6.1-sol` | `gpt-5.4-mini` |

**Altro…** (Other) lets you type any model name your account can use; **Prova** tells you whether it works. The provider and the model are saved with your account and used from the next request.

Both features use the same model.

## The daily budget

diMail stops calling the AI when the day's use reaches 2,000,000 tokens, counted on your Mac from what the provider reports. The Settings page shows where you are: **Oggi … token su …** (Today … tokens out of …).

When the budget is reached, sorting pauses until the next day and diMail says so once; <kbd>⌘J</kbd> answers that the daily limit was reached. The count restarts at midnight, local time. The interface has no control to change the limit yet.

The budget is a safety net, not a price estimate: what you pay depends on your provider and model.

## What is sent to the provider

### When you press ⌘J

- your name and email address;
- the kind of email (new, reply, reply to all, forward), the recipients' names (or addresses, when there is no name) and the subject;
- what you wrote in the composer, if anything;
- for a reply, the thread: its subject and the last four messages, each with the sender's name and address, the date, the text without quoted parts and signatures (cut when long), and the names of its attachments.

### When sections are sorted

For each thread in your inbox that hasn't been checked yet, 20 at a time:

- the participants' names and addresses;
- the subject;
- the snippet, the first few hundred characters that Gmail shows in the list;
- together with the name and the instructions of each of your sections that has instructions.

A thread is checked once per section, and again only if you change that section's instructions.

### Never sent

- Attachments. Only their file names, and only with <kbd>⌘J</kbd>.
- Mail you don't use AI on: with no section instructions, nothing is sent until you press <kbd>⌘J</kbd>.
- Your Google token or anything about your account beyond your name and address.

## What is kept

- The text the AI writes goes into your composer and nowhere else until you send the email.
- A section match becomes a Gmail label on the thread, as if you had applied it yourself.
- diMail's logs record the number of tokens of each request. They never contain email text or the model's answers.

What the provider does with the requests is governed by its terms for API use. Read them before choosing:

- Mistral: [terms](https://mistral.ai/terms/)
- OpenAI: [API data usage](https://openai.com/policies/api-data-usage-policies)

## When something goes wrong

| Message | Meaning |
|---|---|
| **Mistral non accetta questa chiave** (or OpenAI) | The provider rejected the key. Check that you copied all of it and that it wasn't revoked. |
| **manca la chiave di …** | No key is saved for the chosen provider. |
| **… ha risposto 404 con …** | The provider doesn't know that model name, or your account can't use it. Pick one from the list. |
| **… ha risposto 429 con …** | The provider is limiting your account. Wait, or raise your limits in its console. |
| **raggiunto il tetto giornaliero dell'AI: riprova domani** | The [daily budget](#the-daily-budget) was reached. |
| **l'AI non ha scritto niente: riprova** | The model returned an empty answer. Press <kbd>⌘J</kbd> again. |
