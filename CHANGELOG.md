# Changelog

All notable changes to diMail. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [semantic versioning](https://semver.org).

## 0.1.0

The first public release: a Gmail client for the Mac that you drive from the keyboard. The interface is in Italian.

### Added

- **Gmail, directly.** diMail talks to Gmail from your Mac with a Google OAuth client you create yourself, or one your company shares. Mail is kept in a local database; there is no diMail server. See [Creating your Google client](docs/google-client.md).
- **The inbox as a to-do list.** Done (<kbd>E</kbd>), Snooze (<kbd>H</kbd>) with the thread coming back at the top at the time you choose, and a cleanup that marks old threads as done in one undoable step.
- **Important and Other.** The inbox is split on your Mac: mail from your domains and from people you write to stays in Important. <kbd>M</kbd> corrects it for a thread, a sender or a domain.
- **Sections.** Gmail labels with an order, an archive and, optionally, instructions for the AI, which labels the matching emails. A section can be a tab next to Important and Other.
- **Conversations as a chat.** A thread opens on its last message, without quoted text and signatures until you ask; newsletters show as they were sent. Remote images are blocked for senders you don't know.
- **Composer.** New email, reply, reply all and forward, with attachments, your Gmail signature, links made by pasting a URL over selected text, and 30 seconds to undo a send or "Send now".
- **AI with your own key.** Mistral or OpenAI, with the key in the macOS Keychain and the model of your choice. <kbd>⌘J</kbd> writes a reply or tidies up your notes; sections with instructions are sorted in the background; a daily token budget stops runaway use. See [AI](docs/ai.md).
- **Search as you type** over the mail on your Mac, by word prefix and by address.
- **Keyboard first.** Single-key shortcuts for everyday actions, a command palette (<kbd>⌘K</kbd>) and a shortcuts overlay (<kbd>?</kbd>).
- **Contact bar** with what your mail says about a person and your recent conversations with them.
- **Zoom** (<kbd>⌘+</kbd>, <kbd>⌘−</kbd>, <kbd>⌘0</kbd>), light and dark themes, and the number of conversations in Important on the Dock icon.
- **Automatic updates**, signed and verified, that never restart the app by themselves.
- A universal build for Apple silicon and Intel, signed and notarized.
