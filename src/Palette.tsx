// Palette dei comandi (Cmd+K) e aiuto delle scorciatoie (?), dallo stesso registro (sez. 7).
import { useEffect, useMemo, useRef, useState } from "react";
import { fuzzy, keyCaps, type Command } from "./commands";

export type PaletteItem = { id: string; title: string; group: string; keys?: string[]; keywords?: string[]; run: () => void };

const RECENT_KEY = "dimail.palette.recent";

function loadRecent(): string[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
  } catch {
    return [];
  }
}

function remember(id: string) {
  try {
    const r = [id, ...loadRecent().filter((x) => x !== id)].slice(0, 12);
    localStorage.setItem(RECENT_KEY, JSON.stringify(r));
  } catch {
    // Senza localStorage la palette funziona lo stesso, solo senza i recenti.
  }
}

export function Keys({ keys }: { keys: string[] }) {
  // Solo la prima combinazione: le altre sono alternative (frecce, Cmd+Z).
  return <span className="keys">{keyCaps(keys[0]).map((k, i) => <kbd key={i}>{k}</kbd>)}</span>;
}

export function Palette({ items, placeholder, onClose, onCancel }: {
  items: PaletteItem[];
  placeholder?: string;
  /** Dopo aver eseguito una voce. */
  onClose: () => void;
  /** Esc o clic fuori. Se manca, vale onClose. */
  onCancel?: () => void;
}) {
  const cancel = onCancel ?? onClose;
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => input.current?.focus(), []);

  const shown = useMemo(() => {
    if (!q.trim()) {
      const recent = loadRecent();
      const rank = (id: string) => {
        const i = recent.indexOf(id);
        return i < 0 ? 999 : i;
      };
      return [...items].sort((a, b) => rank(a.id) - rank(b.id));
    }
    return items
      .map((it) => ({ it, s: Math.max(fuzzy(q, it.title), ...(it.keywords ?? []).map((k) => fuzzy(q, k) - 1)) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .map((x) => x.it);
  }, [q, items]);

  useEffect(() => setActive(0), [q]);
  useEffect(() => {
    list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const run = (it: PaletteItem | undefined) => {
    if (!it) return;
    remember(it.id);
    onClose();
    it.run();
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, shown.length - 1));
    } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(shown[active]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancel();
    }
  };

  return (
    <>
      <div className="overlay" onClick={cancel} />
      <div className="palette" role="dialog" aria-label="Comandi">
        <input ref={input} value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={onKey}
          placeholder={placeholder ?? "Cerca un comando, un'etichetta, una vista…"} />
        <ul ref={list} role="listbox">
          {shown.map((it, i) => (
            <li key={it.id} className={i === active ? "active" : ""} role="option" aria-selected={i === active}
              onMouseMove={() => setActive(i)} onClick={() => run(it)}>
              <span className="group">{it.group}</span>
              <span className="title">{it.title}</span>
              {it.keys && it.keys.length > 0 && <Keys keys={it.keys} />}
            </li>
          ))}
        </ul>
        {shown.length === 0 && <div className="none">Nessun comando con «{q}».</div>}
      </div>
    </>
  );
}

export function Help({ commands, onClose }: { commands: Command[]; onClose: () => void }) {
  const groups = useMemo(() => {
    const m = new Map<string, Command[]>();
    for (const c of commands) {
      if (c.keys.length === 0 || c.noHelp) continue;
      m.set(c.group, [...(m.get(c.group) ?? []), c]);
    }
    return [...m.entries()];
  }, [commands]);

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "?") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", k, true);
    return () => window.removeEventListener("keydown", k, true);
  }, [onClose]);

  return (
    <>
      <div className="overlay" onClick={onClose} />
      <div className="help" role="dialog" aria-label="Scorciatoie">
        <h3>Scorciatoie</h3>
        <div className="cols">
          {groups.map(([g, cs]) => (
            <div key={g} className="group">
              <h4>{g}</h4>
              {cs.map((c) => (
                <div key={c.id} className="item">
                  <span>{c.title}</span>
                  <Keys keys={c.keys} />
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}
