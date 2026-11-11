// Registro dei comandi (sez. 7 della spec): scorciatoie, palette e aiuto leggono tutti da qui.

export type Context = "list" | "thread";

export type Command = {
  id: string;
  title: string;
  /** Gruppo nell'aiuto e nella palette. */
  group: string;
  /** Tasti: "e", "shift+e", "mod+k", "#", "g i" (sequenza), "arrowdown". */
  keys: string[];
  /** Dove vale. Senza: ovunque. */
  contexts?: Context[];
  keywords?: string[];
  /** Se falso, il comando non c'è (né nella palette né sui tasti). */
  available?: () => boolean;
  run: () => void;
  /** Nascosto dalla palette (per esempio le frecce, doppioni di J e K). */
  hidden?: boolean;
  /** Nascosto dall'aiuto (per esempio i tasti 2–5, già descritti dall'1). */
  noHelp?: boolean;
};

const SYMBOLS: Record<string, string> = {
  mod: "⌘", shift: "⇧", alt: "⌥", enter: "↩", escape: "Esc", tab: "Tab", arrowdown: "↓", arrowup: "↑",
  arrowleft: "←", arrowright: "→", backspace: "⌫", space: "Spazio",
};

/** Per l'interfaccia: "mod+k" → ["⌘", "K"], "g i" → ["G", "I"]. */
export function keyCaps(key: string): string[] {
  return key.split(" ").flatMap((chord) =>
    chord.split("+").map((p) => SYMBOLS[p] ?? (p.length === 1 ? p.toUpperCase() : p)),
  );
}

/** Tasto premuto in forma normale. Per i simboli conta il carattere (su tastiera italiana "#" è Alt+à). */
export function keyOf(e: KeyboardEvent): string | null {
  if (["Shift", "Meta", "Control", "Alt", "CapsLock", "Dead"].includes(e.key)) return null;
  const mod = e.metaKey || e.ctrlKey;
  const k = e.key === " " ? "space" : e.key;
  const parts: string[] = [];
  if (mod) parts.push("mod");
  if (k.length > 1) {
    if (e.shiftKey) parts.push("shift");
    parts.push(k.toLowerCase());
  } else if (/^[a-z]$/i.test(k)) {
    if (e.shiftKey && !mod) parts.push("shift");
    parts.push(k.toLowerCase());
  } else {
    parts.push(k);
  }
  return parts.join("+");
}

function editable(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  return el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

function applies(c: Command, ctx: Context): boolean {
  return (!c.contexts || c.contexts.includes(ctx)) && c.available?.() !== false;
}

/** Instrada i tasti verso i comandi, con le sequenze di due tasti ("g" poi "i"). */
export class KeyRouter {
  private pending: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;

  /** `true` se il tasto è stato usato. */
  handle(e: KeyboardEvent, commands: Command[], ctx: Context): boolean {
    const k = keyOf(e);
    if (!k) return false;
    // Nei campi di testo valgono solo Esc e Cmd+K.
    if (editable(e.target) && k !== "escape" && k !== "mod+k") return false;

    if (this.pending) {
      const seq = `${this.pending} ${k}`;
      this.clear();
      const c = commands.find((c) => c.keys.includes(seq) && applies(c, ctx));
      if (c) {
        c.run();
        return true;
      }
      return false;
    }
    const exact = commands.find((c) => c.keys.includes(k) && applies(c, ctx));
    const starts = commands.some((c) => c.keys.some((x) => x.startsWith(`${k} `)) && applies(c, ctx));
    if (starts && !exact) {
      this.pending = k;
      this.timer = setTimeout(() => this.clear(), 1200);
      return true;
    }
    if (exact) {
      exact.run();
      return true;
    }
    return false;
  }

  clear() {
    this.pending = null;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}

export function available(commands: Command[], ctx: Context): Command[] {
  return commands.filter((c) => applies(c, ctx));
}

/** Minuscole e senza accenti, per cercare "perche" e trovare "perché". */
export function norm(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

/**
 * Ricerca approssimativa: le lettere della query devono comparire in ordine. Vale di più chi
 * le ha all'inizio delle parole e vicine tra loro. 0 = nessuna corrispondenza.
 */
export function fuzzy(query: string, text: string): number {
  const q = norm(query).replace(/\s+/g, "");
  if (!q) return 1;
  const t = norm(text);
  if (t.includes(norm(query).trim())) return 100 - t.indexOf(norm(query).trim());
  let score = 0;
  let ti = 0;
  let last = -2;
  for (const ch of q) {
    const i = t.indexOf(ch, ti);
    if (i < 0) return 0;
    score += 1;
    if (i === 0 || t[i - 1] === " ") score += 3;
    if (i === last + 1) score += 2;
    last = i;
    ti = i + 1;
  }
  return score;
}
