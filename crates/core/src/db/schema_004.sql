-- Inbox divisa: Importanti e Altro (sez. 8 della spec). La colonna threads.split c'è dallo schema 001;
-- qui le correzioni dell'utente e l'indice per le schede. Il primo calcolo lo fa il nucleo all'avvio.

-- Correzioni per singolo thread.
CREATE TABLE split_thread_overrides (
    thread_id   TEXT PRIMARY KEY,
    split       TEXT NOT NULL,           -- important | other
    created_at  INTEGER NOT NULL
);

-- Correzioni ricordate per un mittente o per un dominio.
CREATE TABLE split_rules (
    kind        TEXT NOT NULL,           -- sender | domain
    pattern     TEXT NOT NULL,           -- indirizzo o dominio, minuscolo
    split       TEXT NOT NULL,           -- important | other
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (kind, pattern)
);

CREATE INDEX threads_split ON threads (in_inbox, split, last_date DESC);

-- Da ricalcolare all'avvio.
UPDATE threads SET split = NULL;
