-- Schema v1 (sez. 13), parte per M0-M1. Le fasi successive aggiungono le loro migrazioni
-- (send_queue, jobs, splits, sender_rules, snippets, ai_results).
-- Date in millisecondi Unix. historyId di Gmail come INTEGER.

CREATE TABLE account (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),   -- un file per account
    email               TEXT NOT NULL,
    name                TEXT,
    picture_url         TEXT,
    history_id          INTEGER,          -- punto da cui riparte la sync incrementale
    initial_sync_state  TEXT NOT NULL DEFAULT 'pending',      -- pending | listing | fetching | done
    initial_sync_started_at INTEGER,
    initial_sync_done_at    INTEGER,
    last_sync_at        INTEGER,
    last_sync_error     TEXT,
    messages_total      INTEGER,
    threads_total       INTEGER,
    created_at          INTEGER NOT NULL
);

CREATE TABLE labels (
    id                       TEXT PRIMARY KEY,   -- ID Gmail
    name                     TEXT NOT NULL,
    type                     TEXT NOT NULL,      -- system | user | category
    color_bg                 TEXT,
    color_text               TEXT,
    label_list_visibility    TEXT,
    message_list_visibility  TEXT,
    messages_total           INTEGER,
    messages_unread          INTEGER,
    threads_total            INTEGER,
    threads_unread           INTEGER
);

CREATE TABLE threads (
    id               TEXT PRIMARY KEY,
    history_id       INTEGER NOT NULL,
    subject          TEXT NOT NULL DEFAULT '',
    snippet          TEXT NOT NULL DEFAULT '',
    last_date        INTEGER NOT NULL,           -- data dell'ultimo messaggio (non bozza)
    participants     TEXT NOT NULL DEFAULT '[]', -- JSON [{name, email, unread}] dei mittenti, in ordine
    message_count    INTEGER NOT NULL,
    unread           INTEGER NOT NULL DEFAULT 0,
    starred          INTEGER NOT NULL DEFAULT 0,
    in_inbox         INTEGER NOT NULL DEFAULT 0, -- INBOX e non nel cestino/spam
    important        INTEGER NOT NULL DEFAULT 0,
    has_attachments  INTEGER NOT NULL DEFAULT 0,
    has_draft        INTEGER NOT NULL DEFAULT 0,
    split            TEXT,                       -- divisione dell'inbox calcolata (M3)
    bodies_complete  INTEGER NOT NULL DEFAULT 0, -- tutti i messaggi hanno il corpo in locale
    fetched_at       INTEGER NOT NULL
);
CREATE INDEX threads_inbox ON threads (in_inbox, last_date DESC);
CREATE INDEX threads_last_date ON threads (last_date DESC);

CREATE TABLE thread_labels (
    thread_id  TEXT NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    label_id   TEXT NOT NULL,
    PRIMARY KEY (thread_id, label_id)
) WITHOUT ROWID;
CREATE INDEX thread_labels_by_label ON thread_labels (label_id, thread_id);

CREATE TABLE messages (
    rowid             INTEGER PRIMARY KEY,         -- rowid anche nell'indice FTS
    id                TEXT NOT NULL UNIQUE,        -- ID Gmail
    thread_id         TEXT NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    history_id        INTEGER,
    internal_date     INTEGER NOT NULL,
    label_ids         TEXT NOT NULL DEFAULT '[]',  -- JSON
    snippet           TEXT NOT NULL DEFAULT '',
    from_email        TEXT,
    from_name         TEXT,
    to_json           TEXT NOT NULL DEFAULT '[]',
    cc_json           TEXT NOT NULL DEFAULT '[]',
    bcc_json          TEXT NOT NULL DEFAULT '[]',
    reply_to_json     TEXT NOT NULL DEFAULT '[]',
    subject           TEXT NOT NULL DEFAULT '',
    rfc_message_id    TEXT,                        -- header Message-ID
    in_reply_to       TEXT,
    references_hdr    TEXT,
    list_unsubscribe  TEXT,
    list_id           TEXT,
    precedence        TEXT,
    auto_submitted    TEXT,
    size_estimate     INTEGER,
    has_body          INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX messages_by_thread ON messages (thread_id, internal_date);
CREATE INDEX messages_by_rfc_id ON messages (rfc_message_id);
CREATE INDEX messages_by_from ON messages (from_email);

CREATE TABLE message_bodies (
    message_rowid      INTEGER PRIMARY KEY REFERENCES messages (rowid) ON DELETE CASCADE,
    text_plain         TEXT,
    html_clean         TEXT,
    text_unquoted      TEXT,          -- senza citazioni e firme: per ricerca e AI
    sanitizer_version  INTEGER NOT NULL
);

CREATE TABLE attachments (
    message_rowid        INTEGER NOT NULL REFERENCES messages (rowid) ON DELETE CASCADE,
    part_id              TEXT NOT NULL,
    gmail_attachment_id  TEXT,
    filename             TEXT NOT NULL DEFAULT '',
    mime_type            TEXT NOT NULL DEFAULT '',
    size                 INTEGER NOT NULL DEFAULT 0,
    content_id           TEXT,
    is_inline            INTEGER NOT NULL DEFAULT 0,
    cache_path           TEXT,
    PRIMARY KEY (message_rowid, part_id)
);

-- Contatti ricavati dalla posta (niente directory aziendale nella v1).
CREATE TABLE contacts (
    email             TEXT PRIMARY KEY COLLATE NOCASE,
    name              TEXT,
    sent_count        INTEGER NOT NULL DEFAULT 0,  -- messaggi che ho mandato a questo indirizzo
    received_count    INTEGER NOT NULL DEFAULT 0,  -- messaggi ricevuti da questo indirizzo
    last_sent_at      INTEGER,
    last_received_at  INTEGER,
    source            TEXT NOT NULL DEFAULT 'mail'
);

-- Ricerca a testo pieno (M3 costruisce il linguaggio di query; l'indice si riempie già alla sync).
CREATE VIRTUAL TABLE search_index USING fts5 (
    subject, from_text, to_text, body, attachment_names,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3 4'
);

-- Coda dei thread da scaricare: prima sync, sync incrementale e richieste dell'interfaccia.
-- Persistente, così la prima sync riprende dopo un riavvio.
CREATE TABLE fetch_queue (
    thread_id         TEXT PRIMARY KEY,
    priority          INTEGER NOT NULL,      -- più basso = prima
    want_body         INTEGER NOT NULL,      -- 1 = formato full, 0 = solo metadati
    listed_history_id INTEGER,               -- da threads.list: se in locale è già così recente, si salta
    enqueued_at       INTEGER NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 0,
    not_before        INTEGER NOT NULL DEFAULT 0,
    seq               INTEGER NOT NULL DEFAULT 1   -- cresce a ogni nuova richiesta sullo stesso thread
);
CREATE INDEX fetch_queue_order ON fetch_queue (priority, enqueued_at);

-- Coda delle azioni verso Gmail (sez. 5.4).
CREATE TABLE outbox_ops (
    id               INTEGER PRIMARY KEY,
    kind             TEXT NOT NULL,          -- modify | trash | untrash
    thread_id        TEXT NOT NULL,
    payload          TEXT NOT NULL,          -- JSON
    state            TEXT NOT NULL DEFAULT 'pending',  -- pending | in_flight | done | failed
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  INTEGER NOT NULL,
    last_error       TEXT,
    created_at       INTEGER NOT NULL,
    completed_at     INTEGER
);
CREATE INDEX outbox_due ON outbox_ops (state, next_attempt_at);
CREATE INDEX outbox_by_thread ON outbox_ops (thread_id, state);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL      -- JSON
);
