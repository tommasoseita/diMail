// Sgombero della posta vecchia (specifica UX, sezione 1): conferma con il numero, poi parte e si
// annulla come un'azione sola.
import { useEffect, useState } from "react";
import { api, type BulkPreview } from "./api";
import { count } from "./format";

const DAY = 86_400_000;
const CHOICES: { key: string; label: string; days: number }[] = [
  { key: "7", label: "1 settimana", days: 7 },
  { key: "30", label: "1 mese", days: 30 },
  { key: "90", label: "3 mesi", days: 90 },
  { key: "0", label: "Tutti", days: 0 },
];

export function BulkDialog({ offer, onClose, onStarted }: {
  /** Proposta automatica al primo avvio: il testo lo spiega. */
  offer: boolean;
  onClose: () => void;
  onStarted: (opId: number, threads: number) => void;
}) {
  const [choice, setChoice] = useState("30");
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const days = CHOICES.find((c) => c.key === choice)!.days;

  useEffect(() => {
    let alive = true;
    setPreview(null);
    // "Tutti": fino a domani, così entra anche la posta di oggi.
    const cutoff = days === 0 ? Date.now() + DAY : Date.now() - days * DAY;
    api
      .bulkPreview(cutoff)
      .then((p) => alive && setPreview(p))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [days]);

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", k, true);
    return () => window.removeEventListener("keydown", k, true);
  }, [onClose]);

  const n = preview ? Math.max(preview.local, preview.estimate ?? 0) : null;
  const start = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      const op = await api.bulkDone(preview.cutoff);
      onStarted(op, n ?? preview.local);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <>
      <div className="overlay" onClick={onClose} />
      <div className="dialog" role="dialog" aria-label="Sgombero della posta vecchia">
        <h3>{offer ? "Ripartiamo da zero?" : "Segna come fatti i thread vecchi"}</h3>
        {offer && (
          <p>
            In diMail l'inbox è una lista di cose da fare. Con migliaia di thread vecchi non funziona: segnarli come fatti li
            archivia anche su Gmail, restano cercabili e si annulla tutto con un tasto.
          </p>
        )}
        <div className="choices" role="radiogroup">
          {CHOICES.map((c) => (
            <button key={c.key} className={c.key === choice ? "on" : ""} role="radio" aria-checked={c.key === choice}
              onClick={() => setChoice(c.key)}>
              {c.days === 0 ? c.label : `Più vecchi di ${c.label}`}
            </button>
          ))}
        </div>
        <p className="num">
          {n === null
            ? "Conto i thread…"
            : n === 0
              ? "Nessun thread in inbox è così vecchio."
              : `${preview?.estimate != null && preview.estimate > preview.local ? "Circa " : ""}${count(n)} thread escono dall'inbox.`}
        </p>
        {error && <p className="error small">{error}</p>}
        <div className="buttons">
          <button className="ghost" onClick={onClose}>{offer ? "Non ora" : "Annulla"}</button>
          <button className="primary" disabled={busy || !n} onClick={start}>
            {busy ? "Un momento…" : "Segna come fatti"}
          </button>
        </div>
      </div>
    </>
  );
}
