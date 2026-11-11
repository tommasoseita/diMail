-- Sgombero della posta vecchia (specifica UX, sezione 1): un'operazione su migliaia di thread,
-- annullabile come un'azione sola. In locale vale subito; su Gmail va in background con
-- messages.batchModify (50 unità ogni 1.000 messaggi).
CREATE TABLE bulk_ops (
    id              INTEGER PRIMARY KEY,
    kind            TEXT NOT NULL,             -- done | undo
    query           TEXT NOT NULL,             -- query Gmail, es. "in:inbox before:2026/10/08"
    cutoff          INTEGER NOT NULL,          -- thread con l'ultimo messaggio prima di questa data
    ref_op          INTEGER,                   -- per undo: l'operazione annullata
    state           TEXT NOT NULL,             -- listing | applying | done | cancelled | undone | failed
    threads_local   INTEGER NOT NULL DEFAULT 0,
    messages_total  INTEGER NOT NULL DEFAULT 0,
    messages_done   INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    created_at      INTEGER NOT NULL,
    completed_at    INTEGER
);

-- Thread toccati in locale (per l'annulla).
CREATE TABLE bulk_op_threads (
    op_id      INTEGER NOT NULL REFERENCES bulk_ops (id) ON DELETE CASCADE,
    thread_id  TEXT NOT NULL,
    PRIMARY KEY (op_id, thread_id)
);

-- Messaggi da modificare su Gmail: quelli locali più quelli elencati con la query.
CREATE TABLE bulk_op_messages (
    op_id       INTEGER NOT NULL REFERENCES bulk_ops (id) ON DELETE CASCADE,
    message_id  TEXT NOT NULL,
    applied     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (op_id, message_id)
);
CREATE INDEX bulk_op_messages_todo ON bulk_op_messages (op_id, applied);
