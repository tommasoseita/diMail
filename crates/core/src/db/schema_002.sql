-- Costi di Gmail di maggio 2026 (6.000 unità al minuto per utente, threads.get completo a 120):
-- i thread con un solo messaggio si scaricano con messages.get, che costa la metà.

-- Se c'è, basta scaricare questo messaggio (messages.get) invece del thread intero.
ALTER TABLE fetch_queue ADD COLUMN message_id TEXT;

-- La coda della prima sync era stata costruita con le regole vecchie (inbox intera con i corpi):
-- si ricostruisce. Restano le richieste dell'utente e della sync incrementale.
DELETE FROM fetch_queue WHERE priority >= 20;
UPDATE account SET initial_sync_state = 'listing' WHERE initial_sync_state = 'fetching';
