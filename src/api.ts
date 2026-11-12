// Ponte verso il nucleo Rust. L'interfaccia legge solo da qui: mai Gmail direttamente.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Addr = { name?: string; email: string };
export type Participant = Addr & { unread: boolean };

export type ThreadSummary = {
  id: string;
  subject: string;
  snippet: string;
  last_date: number;
  participants: Participant[];
  message_count: number;
  unread: boolean;
  starred: boolean;
  in_inbox: boolean;
  important: boolean;
  has_attachments: boolean;
  has_draft: boolean;
  bodies_complete: boolean;
  labels: string[];
  /** Tornato da un posticipo: in cima, con il segno "Promemoria", finché non lo si apre. */
  reminded: boolean;
  /** Fino a tre nomi di allegati (non inline) e il totale. */
  attachment_names: string[];
  attachment_count: number;
};

export type AttachmentView = {
  part_id: string;
  filename: string;
  mime_type: string;
  size: number;
  content_id: string | null;
  is_inline: boolean;
};

export type MessageView = {
  id: string;
  date: number;
  from: Addr | null;
  to: Addr[];
  cc: Addr[];
  subject: string;
  snippet: string;
  label_ids: string[];
  has_body: boolean;
  text_plain: string | null;
  html: string | null;
  attachments: AttachmentView[];
  /** Mittente conosciuto: le immagini remote si caricano da sole. */
  trusted_sender: boolean;
  /** Testo senza citazioni né firma, per la conversazione. */
  text_clean: string | null;
  /** Newsletter o invio di massa: si mostra l'email originale. */
  bulk: boolean;
};

export type ThreadDetail = { thread: ThreadSummary; messages: MessageView[] };

export type LabelRow = {
  id: string;
  name: string;
  kind: "system" | "category" | "user";
  color_bg: string | null;
  color_text: string | null;
  label_list_visibility: string | null;
  unread_local: number;
};

export type SyncStatus = {
  phase: "initial" | "syncing" | "idle" | "offline" | "auth_required" | "error" | "";
  initial_sync_state: string;
  queued: number;
  queued_background: number;
  pending_actions: number;
  threads_local: number;
  inbox_local: number;
  last_sync_at: number | null;
  last_error: string | null;
};

export type AccountRow = {
  email: string;
  name: string | null;
  picture_url: string | null;
  history_id: number | null;
  initial_sync_state: string;
  last_sync_at: number | null;
};

export type AppInfo = {
  starting: boolean;
  account: AccountRow | null;
  needs_login: boolean;
  login_hint: string | null;
  /** Manca il client Google: prima schermata, per caricarne il JSON. */
  needs_client: boolean;
};
export type GoogleClientInfo = { client_id: string; project_id: string | null };
/** Cosa c'è in un link di configurazione: `same` se è il client già in uso, `replaces` se ne sostituisce un altro. */
export type SetupLinkInfo = GoogleClientInfo & { same: boolean; replaces: boolean };

export type View =
  | { kind: "inbox" }
  | { kind: "important" }
  | { kind: "other" }
  | { kind: "done" }
  | { kind: "all" }
  | { kind: "search"; q: string }
  | { kind: "label"; id: string }
  | { kind: "section"; id: string };

export type Section = {
  label_id: string;
  name: string;
  position: number;
  archived: boolean;
  show_as_tab: boolean;
  instructions: string | null;
  color_bg: string | null;
  inbox: number;
  unread: number;
};

export type Draft = {
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  subject: string;
  body: string;
  thread_id: string | null;
  in_reply_to: string | null;
  references: string | null;
  quoted: string | null;
  attachments: OutAttachment[];
  /** Firma di Gmail tolta per questa email. */
  no_signature?: boolean;
};

/** Allegato di un'email da inviare: un file del Mac, o (inoltri) un allegato del messaggio originale. */
export type OutAttachment = {
  name: string;
  size: number;
  mime_type: string;
  path?: string | null;
  message_id?: string | null;
  part_id?: string | null;
};

/** La firma di Gmail web. */
export type Signature = { html: string; text: string };

export type ReplyMode = "reply" | "reply_all" | "forward";
export type Queued = { id: number; send_at: number };
export type SnoozeHandle = { ops: number[]; jobs: number[] };
export type Snoozed = { job_id: number; thread_id: string; due_at: number };
export type AiWriteRequest = { mode: string; thread_id: string | null; to: Addr[]; subject: string; notes: string };
export type Provider = "mistral" | "openai";
export const PROVIDER_NAME: Record<Provider, string> = { mistral: "Mistral", openai: "OpenAI" };
export type AiStatus = { has_key: boolean; provider: Provider; model: string; keys: Provider[]; tokens_today: number; daily_cap: number };
export type ModelInfo = { provider: Provider; id: string; name: string; recommended: boolean };
export type MyDomains = { domains: string[]; custom: boolean; suggested: string | null };

export type Split = "important" | "other";
export type Remember = "thread" | "sender" | "domain";
export type SplitRule = { kind: "sender" | "domain"; pattern: string; split: Split; created_at: number };

export type ContactCard = {
  email: string;
  name: string | null;
  company: string | null;
  role: string | null;
  phone: string | null;
  received_count: number;
  sent_count: number;
  first_date: number | null;
  last_date: number | null;
  recent: ThreadSummary[];
};

export type BulkPreview = { cutoff: number; query: string; local: number; estimate: number | null };

export type BulkStatus = { op_id: number; kind: "done" | "undo"; state: string; done: number; total: number };

export type Action =
  | { type: "archive" }
  | { type: "move_to_inbox" }
  | { type: "mark_read" }
  | { type: "mark_unread" }
  | { type: "star" }
  | { type: "unstar" }
  | { type: "trash" }
  | { type: "untrash" }
  | { type: "add_label"; label_id: string }
  | { type: "remove_label"; label_id: string };

export type CoreEvent =
  | { type: "threads_changed"; ids: string[] }
  | { type: "labels_changed" }
  | ({ type: "sync_status" } & SyncStatus)
  | { type: "action_failed"; thread_id: string | null; message: string }
  | { type: "auth_required"; email: string }
  | ({ type: "bulk_status" } & BulkStatus)
  | { type: "send_status"; id: number; state: "sent" | "retrying" | "failed"; error: string | null }
  | { type: "ai_status"; state: string };

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  login: () => invoke<AccountRow>("login"),
  googleClient: () => invoke<GoogleClientInfo | null>("google_client"),
  /** Senza percorso apre il pannello di macOS; `null` se l'utente annulla. */
  loadGoogleClient: (path?: string) => invoke<GoogleClientInfo | null>("load_google_client", { path: path ?? null }),
  /** Il link di configurazione del client in uso (`dimail://client?c=…`), da mandare ai colleghi. */
  clientSetupLink: () => invoke<string>("client_setup_link"),
  setupLinkInfo: (link: string) => invoke<SetupLinkInfo>("setup_link_info", { link }),
  applySetupLink: (link: string) => invoke<GoogleClientInfo>("apply_setup_link", { link }),
  listThreads: (view: View, limit = 300, offset = 0) => invoke<ThreadSummary[]>("list_threads", { view, limit, offset }),
  countThreads: (view: View) => invoke<number>("count_threads", { view }),
  /** Come getThread, ma senza chiedere i corpi mancanti: per precaricare. */
  peekThread: (id: string) => invoke<ThreadDetail | null>("peek_thread", { id }),
  moveSplit: (threadIds: string[], split: Split, remember: Remember) => invoke<void>("move_split", { threadIds, split, remember }),
  splitRules: () => invoke<SplitRule[]>("split_rules"),
  removeSplitRule: (kind: string, pattern: string) => invoke<void>("remove_split_rule", { kind, pattern }),
  contactCard: (email: string) => invoke<ContactCard>("contact_card", { email }),
  doneToday: (since: number) => invoke<number>("done_today", { since }),
  getSetting: <T>(key: string) => invoke<T | null>("get_setting", { key }),
  setSetting: (key: string, value: unknown) => invoke<void>("set_setting", { key, value }),
  bulkPreview: (cutoff: number) => invoke<BulkPreview>("bulk_preview", { cutoff }),
  bulkDone: (cutoff: number) => invoke<number>("bulk_done", { cutoff }),
  bulkUndo: (opId: number) => invoke<number>("bulk_undo", { opId }),
  getThread: (id: string) => invoke<ThreadDetail | null>("get_thread", { id }),
  listLabels: () => invoke<LabelRow[]>("list_labels"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  act: (threadIds: string[], action: Action) => invoke<number[]>("act", { threadIds, action }),
  undo: (opIds: number[]) => invoke<number[]>("undo", { opIds }),
  syncNow: () => invoke<void>("sync_now"),
  snooze: (threadIds: string[], dueAt: number) => invoke<SnoozeHandle>("snooze", { threadIds, dueAt }),
  unsnooze: (handle: SnoozeHandle) => invoke<void>("unsnooze", { handle }),
  listSnoozed: () => invoke<Snoozed[]>("list_snoozed"),
  clearReminder: (threadId: string) => invoke<void>("clear_reminder", { threadId }),
  replyDraft: (threadId: string, mode: ReplyMode) => invoke<Draft>("reply_draft", { threadId, mode }),
  sendDraft: (draft: Draft, delayMs: number) => invoke<Queued>("send_draft", { draft, delayMs }),
  cancelSend: (id: number) => invoke<Draft | null>("cancel_send", { id }),
  sendNow: (id: number) => invoke<boolean>("send_now", { id }),
  suggestContacts: (q: string) => invoke<Addr[]>("suggest_contacts", { q }),
  aiStatus: () => invoke<AiStatus>("ai_status"),
  aiSetKey: (provider: Provider, key: string) => invoke<void>("ai_set_key", { provider, key }),
  aiClearKey: (provider: Provider) => invoke<void>("ai_clear_key", { provider }),
  aiModels: () => invoke<ModelInfo[]>("ai_models"),
  aiConfigure: (provider: Provider, model: string) => invoke<void>("ai_configure", { provider, model }),
  aiCheck: () => invoke<number>("ai_check"),
  myDomains: () => invoke<MyDomains>("my_domains"),
  setMyDomains: (domains: string[]) => invoke<MyDomains>("set_my_domains", { domains }),
  /** Scarica l'allegato (la prima volta) e lo apre con l'app predefinita del Mac. */
  openAttachment: (messageId: string, partId: string) => invoke<void>("open_attachment", { messageId, partId }),
  /** Copia l'allegato in Download: restituisce il percorso. */
  saveAttachment: (messageId: string, partId: string) => invoke<string>("save_attachment", { messageId, partId }),
  /** Un'immagine nel corpo dell'email (`cid:`), come indirizzo `data:`. */
  inlineImage: (messageId: string, partId: string) => invoke<string>("inline_image", { messageId, partId }),
  signature: () => invoke<Signature | null>("get_signature"),
  pickAttachments: () => invoke<OutAttachment[]>("pick_attachments"),
  fileInfo: (paths: string[]) => invoke<OutAttachment[]>("file_info", { paths }),
  revealDownload: (path: string) => invoke<void>("reveal_download", { path }),
  /** ⌘J: scrive la risposta o sistema la bozza. */
  aiWrite: (req: AiWriteRequest) => invoke<string>("ai_write", { req }),
  listSections: () => invoke<Section[]>("list_sections"),
  createSection: (name: string, showAsTab: boolean, instructions: string | null) =>
    invoke<Section>("create_section", { name, showAsTab, instructions }),
  renameSection: (labelId: string, name: string) => invoke<void>("rename_section", { labelId, name }),
  configureSection: (labelId: string, showAsTab: boolean, instructions: string | null) =>
    invoke<void>("configure_section", { labelId, showAsTab, instructions }),
  archiveSection: (labelId: string, archived: boolean) => invoke<void>("archive_section", { labelId, archived }),
  reorderSections: (ordered: string[]) => invoke<void>("reorder_sections", { ordered }),
};

export const emptyDraft = (): Draft => ({
  to: [],
  cc: [],
  bcc: [],
  subject: "",
  body: "",
  thread_id: null,
  in_reply_to: null,
  references: null,
  quoted: null,
  attachments: [],
});

/// L'account è stato ripreso (o non c'era un token): l'interfaccia rilegge `app_info`.
export function onAppReady(handler: () => void): Promise<UnlistenFn> {
  return listen("app_ready", () => handler());
}

export function onCoreEvent(handler: (e: CoreEvent) => void): Promise<UnlistenFn> {
  return listen<CoreEvent>("core", (e) => handler(e.payload));
}
