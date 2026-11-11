// Posticipa (H): il thread esce dall'inbox e torna in cima all'ora scelta. Scelte rapide con i
// numeri, oppure si scrive "domani alle 15", "venerdì", "tra 2 ore", "20/11 9:30".
import { useEffect, useMemo, useRef, useState } from "react";
import { norm } from "./commands";

const HOUR = 3_600_000;
const MORNING = 8;

const WEEKDAYS = ["domenica", "lunedi", "martedi", "mercoledi", "giovedi", "venerdi", "sabato"];

function at(d: Date, h: number, m = 0): Date {
  const x = new Date(d);
  x.setHours(h, m, 0, 0);
  return x;
}

function addDays(d: Date, n: number): Date {
  const x = new Date(d);
  x.setDate(x.getDate() + n);
  return x;
}

/** Il prossimo giorno della settimana (0 = domenica), mai oggi. */
function nextWeekday(from: Date, wd: number): Date {
  const diff = (wd - from.getDay() + 7) % 7 || 7;
  return addDays(from, diff);
}

export function snoozeLabel(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  const time = d.toLocaleTimeString("it-IT", { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === today.toDateString()) return `oggi alle ${time}`;
  if (d.toDateString() === addDays(today, 1).toDateString()) return `domani alle ${time}`;
  const day = d.toLocaleDateString("it-IT", { weekday: "long", day: "numeric", month: "long" });
  return `${day} alle ${time}`;
}

/** Testo libero in italiano → data, o null. */
export function parseWhen(input: string, now = new Date()): Date | null {
  const q = norm(input).trim().replace(/\s+/g, " ");
  if (!q) return null;

  // "tra 2 ore", "fra 3 giorni", "tra 30 minuti", "tra una settimana"
  const rel = q.match(/^(?:tra|fra|in)\s+(un|una|\d+)\s*(minut[oi]|min|or[ae]|h|giorn[oi]|g|settiman[ae]|mes[ei])$/);
  if (rel) {
    const n = rel[1] === "un" || rel[1] === "una" ? 1 : Number(rel[1]);
    const u = rel[2];
    if (u.startsWith("min")) return new Date(now.getTime() + n * 60_000);
    if (u.startsWith("or") || u === "h") return new Date(now.getTime() + n * HOUR);
    if (u.startsWith("giorn") || u === "g") return at(addDays(now, n), MORNING);
    if (u.startsWith("settiman")) return at(addDays(now, 7 * n), MORNING);
    const m = new Date(now);
    m.setMonth(m.getMonth() + n);
    return at(m, MORNING);
  }

  // Ora in fondo: "alle 15", "15:30", "ore 9", "h 9"
  let hour: number | null = null;
  let minute = 0;
  let rest = q;
  const t = q.match(/(?:^|\s)(?:alle|ore|h|a)?\s*(\d{1,2})(?:[:.](\d{2}))?$/);
  const dateOnly = /^\d{1,2}[/.-]\d{1,2}([/.-]\d{2,4})?$/.test(q);
  if (t && !dateOnly && (q.includes(":") || q.includes(".") || /(alle|ore|h)\s*\d/.test(q) || /^\d{1,2}$/.test(q))) {
    hour = Number(t[1]);
    minute = t[2] ? Number(t[2]) : 0;
    if (hour > 23 || minute > 59) return null;
    rest = q.slice(0, t.index).trim();
  }

  let day: Date | null = null;
  if (!rest || rest === "oggi") day = now;
  else if (rest === "stasera") { day = now; hour ??= 18; }
  else if (rest === "domani") day = addDays(now, 1);
  else if (rest === "dopodomani") day = addDays(now, 2);
  else if (rest === "weekend" || rest === "fine settimana" || rest === "questo weekend") { day = nextWeekday(now, 6); hour ??= 9; }
  else if (rest === "settimana prossima" || rest === "la settimana prossima" || rest === "prossima settimana") day = nextWeekday(now, 1);
  else if (rest === "mese prossimo" || rest === "il mese prossimo") { day = new Date(now.getFullYear(), now.getMonth() + 1, 1); }
  else {
    const wd = WEEKDAYS.findIndex((w) => rest === w || rest === w.slice(0, 3) || rest === `${w} prossimo` || rest === `prossimo ${w}`);
    if (wd >= 0) day = nextWeekday(now, wd);
    else {
      const dm = rest.match(/^(\d{1,2})[/.-](\d{1,2})(?:[/.-](\d{2,4}))?$/);
      if (dm) {
        let y = dm[3] ? Number(dm[3]) : now.getFullYear();
        if (y < 100) y += 2000;
        day = new Date(y, Number(dm[2]) - 1, Number(dm[1]));
        if (day.getDate() !== Number(dm[1])) return null;
        if (!dm[3] && at(day, hour ?? MORNING, minute) <= now) day.setFullYear(y + 1);
      }
    }
  }
  if (!day) return null;
  const when = at(day, hour ?? (day === now ? now.getHours() + 3 : MORNING), hour === null && day === now ? 0 : minute);
  // Solo un'ora, già passata: domani a quell'ora.
  if (when <= now && !rest) return addDays(when, 1);
  return when > now ? when : null;
}

type Choice = { id: string; title: string; when: Date };

function presets(now = new Date()): Choice[] {
  const out: Choice[] = [];
  const h = now.getHours();
  if (h < 20) {
    const later = new Date(now.getTime() + 3 * HOUR);
    later.setMinutes(later.getMinutes() < 30 ? 30 : 60, 0, 0);
    out.push({ id: "later", title: "Più tardi oggi", when: later });
  }
  if (h < 17) out.push({ id: "evening", title: "Stasera", when: at(now, 18) });
  out.push({ id: "tomorrow", title: "Domani mattina", when: at(addDays(now, 1), MORNING) });
  const wd = now.getDay();
  if (wd >= 1 && wd <= 4) out.push({ id: "weekend", title: "Questo fine settimana", when: at(nextWeekday(now, 6), 9) });
  out.push({ id: "monday", title: wd === 0 || wd >= 5 ? "Lunedì mattina" : "La settimana prossima", when: at(nextWeekday(now, 1), MORNING) });
  out.push({ id: "month", title: "Tra un mese", when: (() => { const m = new Date(now); m.setMonth(m.getMonth() + 1); return at(m, MORNING); })() });
  return out;
}

export function SnoozePicker({ count, onPick, onClose }: { count: number; onPick: (when: number, label: string) => void; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);

  const items = useMemo<Choice[]>(() => {
    const base = presets();
    if (!q.trim()) return base;
    const parsed = parseWhen(q);
    const matches = base.filter((c) => norm(c.title).includes(norm(q)));
    return parsed ? [{ id: "custom", title: q.trim(), when: parsed }, ...matches] : matches;
  }, [q]);

  useEffect(() => setActive(0), [q]);

  const pick = (c: Choice | undefined) => {
    if (!c) return;
    onPick(c.when.getTime(), snoozeLabel(c.when.getTime()));
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, items.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(items[active]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (!q && /^[1-9]$/.test(e.key) && Number(e.key) <= items.length) {
      e.preventDefault();
      pick(items[Number(e.key) - 1]);
    }
  };

  return (
    <>
      <div className="overlay" onClick={onClose} />
      <div className="palette snooze" role="dialog" aria-label="Posticipa">
        <div className="snooze-head">Posticipa {count > 1 ? `${count} thread` : "il thread"}: torna in cima all'ora scelta</div>
        <input ref={input} value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={onKey}
          placeholder="Quando? Domani alle 15, venerdì, tra 2 ore, 20/11 9:30…" />
        <ul role="listbox">
          {items.map((c, i) => (
            <li key={c.id} className={i === active ? "active" : ""} role="option" aria-selected={i === active}
              onMouseMove={() => setActive(i)} onClick={() => pick(c)}>
              <span className="title">{c.id === "custom" ? snoozeLabel(c.when.getTime()) : c.title}</span>
              <span className="when muted">{c.id === "custom" ? "" : snoozeLabel(c.when.getTime())}</span>
              {!q && i < 9 && <span className="keys"><kbd>{i + 1}</kbd></span>}
            </li>
          ))}
        </ul>
        {items.length === 0 && <div className="none">Non capisco «{q}». Prova «domani alle 9» o «20/11».</div>}
      </div>
    </>
  );
}
