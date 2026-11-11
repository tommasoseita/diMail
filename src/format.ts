import type { Addr, Participant } from "./api";

const time = new Intl.DateTimeFormat("it-IT", { hour: "2-digit", minute: "2-digit" });
const dayMonth = new Intl.DateTimeFormat("it-IT", { day: "numeric", month: "short" });
const full = new Intl.DateTimeFormat("it-IT", { day: "2-digit", month: "2-digit", year: "2-digit" });
const long = new Intl.DateTimeFormat("it-IT", { weekday: "short", day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });

export function shortDate(ms: number): string {
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return time.format(d);
  if (d.getFullYear() === now.getFullYear()) return dayMonth.format(d);
  return full.format(d);
}

export function longDate(ms: number): string {
  return long.format(new Date(ms));
}

export function clock(ms: number): string {
  return time.format(new Date(ms));
}

export function addrLabel(a: Addr | null | undefined, me?: string): string {
  if (!a) return "(sconosciuto)";
  if (me && a.email === me) return "io";
  return a.name || a.email;
}

function firstName(p: Participant, me: string): string {
  if (p.email === me) return "io";
  const n = p.name || p.email.split("@")[0];
  return n.split(/\s+/)[0];
}

export function participantsLabel(ps: Participant[], me: string): string {
  if (ps.length === 0) return "(bozza)";
  if (ps.length === 1) return ps[0].email === me ? "io" : ps[0].name || ps[0].email;
  const names = ps.map((p) => firstName(p, me));
  if (names.length <= 3) return names.join(", ");
  return `${names[0]} … ${names.slice(-2).join(", ")}`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

const numbers = new Intl.NumberFormat("it-IT");
export function count(n: number): string {
  return numbers.format(n);
}

/** "Anna Rossi" → "AR", "anna@x.it" → "A". */
export function initials(name: string | null | undefined, email: string): string {
  const src = (name || email.split("@")[0]).replace(/[^\p{L}\s]/gu, " ").trim();
  const words = src.split(/\s+/).filter(Boolean);
  const s = words.length >= 2 ? words[0][0] + words[words.length - 1][0] : (words[0] ?? email).slice(0, 1);
  return s.toUpperCase();
}

/** La persona di un thread per la barra del contatto: l'ultimo mittente che non sono io. */
export function personOf(ps: Participant[], me: string, fallback?: Addr | null): Addr | null {
  for (let i = ps.length - 1; i >= 0; i--) if (ps[i].email !== me) return ps[i];
  return fallback && fallback.email !== me ? fallback : null;
}

/** Mezzanotte di oggi, ora locale. */
export function startOfToday(): number {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** "Oggi 14:05", "Ieri 9:10", "7 nov", "7/11/24": per le schede della lista. */
export function whenLabel(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return `Oggi ${time.format(d)}`;
  if (d.toDateString() === yesterday.toDateString()) return `Ieri ${time.format(d)}`;
  if (d.getFullYear() === today.getFullYear()) return dayMonth.format(d);
  return full.format(d);
}

/** "9:41" oggi, altrimenti "7 nov, 9:41": per la conversazione. */
export function chatTime(ms: number): string {
  const d = new Date(ms);
  if (d.toDateString() === new Date().toDateString()) return time.format(d);
  return `${dayMonth.format(d)}, ${time.format(d)}`;
}

const AVATAR_COLORS = ["#5b8def", "#e0805a", "#4fae7b", "#9a74d8", "#d4a03b", "#3fa7b5", "#d9667f", "#7c8a99"];
/** Colore dell'avatar, sempre lo stesso per lo stesso indirizzo. */
export function avatarColor(email: string): string {
  let h = 0;
  for (const ch of email) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

const monthName = new Intl.DateTimeFormat("it-IT", { month: "long" });

/** Gruppo della lista per data: Oggi, Ieri, Ultimi 7 giorni, poi i mesi ("Ottobre", "Settembre 2025"). */
export function dayGroup(ms: number, today = startOfToday()): { key: string; label: string } {
  const DAY = 86_400_000;
  if (ms >= today) return { key: "today", label: "Oggi" };
  if (ms >= today - DAY) return { key: "yesterday", label: "Ieri" };
  if (ms >= today - 6 * DAY) return { key: "week", label: "Ultimi 7 giorni" };
  const d = new Date(ms);
  const m = monthName.format(d);
  const label = m.charAt(0).toUpperCase() + m.slice(1) + (d.getFullYear() === new Date(today).getFullYear() ? "" : ` ${d.getFullYear()}`);
  return { key: `${d.getFullYear()}-${d.getMonth()}`, label };
}
