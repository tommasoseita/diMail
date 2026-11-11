// Sezioni: crea o modifica. Una sezione è un'etichetta di Gmail con, in più, le istruzioni per l'AI
// (cosa deve finire lì) e la scelta di mostrarla come scheda accanto a Importanti e Altro.
import { useEffect, useRef, useState } from "react";
import type { Section } from "./api";
import { Sparkle } from "./icons";

type Props = {
  section: Section | null;
  defaultTab: boolean;
  aiOn: boolean;
  onSave: (v: { name: string; instructions: string | null; showAsTab: boolean }) => Promise<void>;
  onSetupAi: () => void;
  onClose: () => void;
};

export function SectionDialog({ section, defaultTab, aiOn, onSave, onSetupAi, onClose }: Props) {
  const [name, setName] = useState(section?.name ?? "");
  const [instructions, setInstructions] = useState(section?.instructions ?? "");
  const [tab, setTab] = useState(section?.show_as_tab ?? defaultTab);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => nameRef.current?.focus(), []);

  const save = async () => {
    if (!name.trim()) {
      setError("Serve un nome.");
      nameRef.current?.focus();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onSave({ name: name.trim(), instructions: instructions.trim() || null, showAsTab: tab });
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey || (e.target as HTMLElement).tagName === "INPUT")) {
      e.preventDefault();
      save();
    }
  };

  return (
    <>
      <div className="overlay" onClick={onClose} />
      <div className="dialog section-dialog" role="dialog" aria-label={section ? "Modifica la sezione" : "Nuova sezione"} onKeyDown={onKey}>
        <h3>{section ? "Modifica la sezione" : "Nuova sezione"}</h3>
        <p className="muted small">È un'etichetta di Gmail: la vedi anche su Gmail web e sul telefono.</p>
        <label className="form-row">
          <span>Nome</span>
          <input ref={nameRef} value={name} onChange={(e) => setName(e.target.value)} placeholder="Fornitori" spellCheck={false} />
        </label>
        <label className="form-row">
          <span className="with-icon"><Sparkle size={15} style={{ color: "var(--ai)" }} /> Cosa deve finire qui</span>
          <textarea value={instructions} onChange={(e) => setInstructions(e.target.value)} rows={4}
            placeholder="Es. fatture, preventivi e conferme d'ordine dei fornitori, anche quando scrivono da indirizzi personali." />
          <span className="hint">
            {aiOn
              ? "L'AI legge le email in inbox e porta qui quelle che corrispondono, anche quelle che arriveranno. Lasciala vuota per una sezione a mano."
              : <>Per lo smistamento automatico serve la chiave dell'AI. <button className="link-btn" onClick={onSetupAi}>Impostala</button></>}
          </span>
        </label>
        <label className="check-row">
          <input type="checkbox" checked={tab} onChange={(e) => setTab(e.target.checked)} />
          <span>Mostra come scheda accanto a Importanti e Altro</span>
        </label>
        {error && <p className="error small">{error}</p>}
        <div className="buttons">
          <button className="ghost" onClick={onClose}>Annulla</button>
          <button className="primary" onClick={save} disabled={busy}>{busy ? "Salvo…" : section ? "Salva" : "Crea la sezione"}</button>
        </div>
      </div>
    </>
  );
}
