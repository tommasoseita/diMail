-- 9 novembre 2026: Posticipa, composer di base, AI con Mistral, sezioni.

-- Lavori a tempo (sez. 10 della spec). Per ora solo Posticipa ("snooze"). Lo stato passa da in_attesa
-- a in_esecuzione in modo atomico, così un lavoro non parte due volte.
CREATE TABLE jobs (
    id            INTEGER PRIMARY KEY,
    kind          TEXT NOT NULL,                 -- snooze
    thread_id     TEXT NOT NULL,
    due_at        INTEGER NOT NULL,
    state         TEXT NOT NULL,                 -- in_attesa | in_esecuzione | completato | fallito | annullato
    payload       TEXT NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    completed_at  INTEGER
);
CREATE INDEX jobs_due ON jobs (state, due_at);

-- Quando un posticipato torna: in cima alla lista finché non lo si tocca.
ALTER TABLE threads ADD COLUMN reminded_at INTEGER;

-- Coda di invio (sez. 10): il messaggio resta qui per il tempo dell'annulla. Il Message-ID è generato
-- qui: prima di ripetere un invio fallito si controlla negli Inviati che non sia già partito.
CREATE TABLE send_queue (
    id              INTEGER PRIMARY KEY,
    draft           TEXT NOT NULL,               -- JSON della bozza, per riaprirla dopo un annulla
    raw             TEXT NOT NULL,               -- RFC 2822 in base64url
    thread_id       TEXT,
    rfc_message_id  TEXT NOT NULL,
    send_at         INTEGER NOT NULL,
    state           TEXT NOT NULL,               -- pending | sending | sent | cancelled | failed
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    gmail_id        TEXT,
    created_at      INTEGER NOT NULL,
    sent_at         INTEGER
);
CREATE INDEX send_queue_due ON send_queue (state, send_at);

-- Sintesi AI dei thread. Si cancella quando arriva un messaggio nuovo nel thread.
CREATE TABLE ai_summaries (
    thread_id   TEXT PRIMARY KEY,
    summary     TEXT NOT NULL,
    model       TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

-- Consumo dell'AI per giorno, per il tetto di spesa.
CREATE TABLE ai_usage (
    day       TEXT PRIMARY KEY,                  -- AAAA-MM-GG, ora locale
    tokens    INTEGER NOT NULL DEFAULT 0,
    requests  INTEGER NOT NULL DEFAULT 0
);

-- Sezioni: le etichette di Gmail nella barra a sinistra, con ordine, archivio, scheda accanto a
-- Importanti e Altro, e istruzioni per l'AI che le assegna da sola.
CREATE TABLE sections (
    label_id      TEXT PRIMARY KEY,
    position      INTEGER NOT NULL DEFAULT 0,
    archived      INTEGER NOT NULL DEFAULT 0,
    show_as_tab   INTEGER NOT NULL DEFAULT 0,
    instructions  TEXT,
    created_at    INTEGER NOT NULL
);

-- Thread già valutati dall'AI per una sezione: non si chiede due volte.
CREATE TABLE section_checks (
    label_id    TEXT NOT NULL,
    thread_id   TEXT NOT NULL,
    matched     INTEGER NOT NULL,
    checked_at  INTEGER NOT NULL,
    PRIMARY KEY (label_id, thread_id)
);
