// Backend finto per sviluppare l'interfaccia nel browser (`npm run dev`), senza Tauri e senza toccare
// la posta vera né la quota Gmail. Si carica solo in sviluppo e fuori da Tauri (main.tsx): nella
// build non c'è. Imita i comandi del nucleo e i suoi eventi, con dati inventati.

type Lbl = string;
type Msg = { id: string; date: number; from: Person; to: Person[]; cc?: Person[]; labels: Lbl[]; html: string | null; text: string; snippet: string; attach?: string; inline?: { cid: string; name: string }[] };
type Th = { id: string; subject: string; msgs: Msg[] };
type Person = { name: string; email: string; role?: string; phone?: string };

const ME: Person = { name: "Tommaso Seita", email: "tommaso@esempio.it" };
const DAY = 86_400_000;
const now = Date.now();

// Generatore deterministico: gli stessi dati a ogni ricarica.
let seed = 42;
const rnd = () => ((seed = (seed * 1664525 + 1013904223) % 4294967296) / 4294967296);
const pick = <T,>(a: T[]) => a[Math.floor(rnd() * a.length)];

const PEOPLE: Person[] = [
  { name: "Anna Rossi", email: "anna.rossi@cliente-srl.it", role: "Responsabile acquisti", phone: "+39 011 123 4567" },
  { name: "Luca Bianchi", email: "luca@esempio.it", role: "Product designer" },
  { name: "Sara Conti", email: "sara.conti@studiolegale.it", role: "Avvocato", phone: "+39 02 765 4321" },
  { name: "Marco Ferri", email: "marco.ferri@gmail.com" },
  { name: "Giulia Neri", email: "giulia@agenziablu.com", role: "Account manager", phone: "333 1234567" },
  { name: "Paolo Greco", email: "p.greco@fornitore.eu", role: "Amministrazione" },
  { name: "Elena Galli", email: "elena@esempio.it", role: "Operations" },
  { name: "Newsletter Prodotto", email: "news@prodotto.io" },
  { name: "Banca Esempio", email: "noreply@banca-esempio.it" },
  { name: "Davide Costa", email: "davide.costa@investitori.vc", role: "Partner", phone: "+39 347 555 0101" },
];
const SUBJECTS = [
  "Preventivo per il progetto di formazione", "Riunione di lunedì", "Contratto da firmare", "Fattura di ottobre",
  "Domanda veloce sul calendario", "Proposta di collaborazione", "Aggiornamento sul rilascio", "Documenti per la due diligence",
  "Invito: workshop AI per i manager", "Recap della call", "Nuovo accesso al tuo account", "Pagamento ricevuto",
  "Richiesta di informazioni", "Feedback sulla demo", "Ordine confermato", "Rinnovo dell'abbonamento",
];
const LINES = [
  "Ti mando la versione aggiornata con le modifiche che abbiamo discusso.", "Ci vediamo alle 10, va bene anche online.",
  "Fammi sapere se ti torna, così procediamo.", "In allegato trovi il documento da firmare.", "Grazie mille, ci aggiorniamo la settimana prossima.",
  "Ho visto la demo: molto bene, due note sul flusso di login.", "Confermo la disponibilità per giovedì pomeriggio.",
];

function body(from: Person, i: number, quoteOf?: Msg): { html: string | null; text: string } {
  const line = pick(LINES);
  const sig = [from.name, from.role, from.phone ? `Tel. ${from.phone}` : null].filter(Boolean).join("\n");
  const text = `Ciao Tommaso,\n\n${line}\n\nA presto,\n${sig}`;
  if (i % 3 === 0) return { html: null, text: quoteOf ? `${text}\n\nIl giorno ${new Date(quoteOf.date).toLocaleDateString("it-IT")} ${quoteOf.from.name} ha scritto:\n> ${quoteOf.snippet}` : text };
  const img = from.email.startsWith("news") ? `<p><img src="https://picsum.photos/seed/${i}/560/180" alt=""></p>` : "";
  const quote = quoteOf ? `<div class="gmail_quote"><p>Il ${new Date(quoteOf.date).toLocaleDateString("it-IT")} ${quoteOf.from.name} ha scritto:</p><blockquote>${quoteOf.snippet}</blockquote></div>` : "";
  const html = `${img}<p>Ciao Tommaso,</p><p>${line} <a href="https://esempio.it">esempio.it</a></p><p>A presto,<br>${sig.replace(/\n/g, "<br>")}</p>${quote}`;
  return { html, text };
}

const threads: Th[] = [];
for (let n = 0; n < 3300; n++) {
  // Più posta recente che vecchia: età in giorni con distribuzione sbilanciata.
  const age = Math.floor(Math.pow(rnd(), 2.2) * 730 * DAY);
  const start = now - age - Math.floor(rnd() * DAY);
  const other = pick(PEOPLE);
  const count = rnd() < 0.75 ? 1 : 2 + Math.floor(rnd() * 3);
  const inbox = n < 2600 || rnd() < 0.1;
  const msgs: Msg[] = [];
  for (let i = 0; i < count; i++) {
    const mine = i % 2 === 1;
    const from = mine ? ME : other;
    const b = body(from, n + i, msgs[i - 1]);
    const date = Math.min(now - (n * 13 + (count - i) * 47) * 60_000, start + i * Math.floor(rnd() * DAY * 2));
    const labels: Lbl[] = mine ? ["SENT"] : [...(inbox ? ["INBOX"] : []), ...(age < 4 * DAY && rnd() < 0.5 ? ["UNREAD"] : [])];
    if (!mine && other.email.startsWith("news")) labels.push("CATEGORY_PROMOTIONS");
    if (!mine && rnd() < 0.04) labels.push("STARRED");
    if (!mine && n % 37 === 0) labels.push("Label_1");
    msgs.push({ id: `m${n}-${i}`, date, from, to: [mine ? other : ME], labels, html: b.html, text: b.text,
      snippet: b.text.split("\n").slice(2, 3).join(" ").slice(0, 120), attach: rnd() < 0.1 ? "preventivo.pdf" : undefined });
  }
  threads.push({ id: `t${n}`, subject: pick(SUBJECTS), msgs });
}

// Un messaggio scritto da Gmail come quelli veri: elenco, firma con logo, testo citato.
{
  const html = `<div dir="ltr"><p style="font-family:Arial;font-size:13px;color:#222">Ciao Giulia, di seguito le cose di cui avremmo bisogno:</p>
<ul><li><font color="#000000">numero e data del preventivo firmato</font></li><li>elenco dei partecipanti al corso, con ruolo;</li>
<li>indirizzo della sede e orari in cui la sala è libera;</li><li>referente per la parte tecnica, con <b>telefono</b>;</li></ul>
<div><br></div><div>Grazie!</div><div><br></div>
<div class="gmail_signature" data-smartmail="gmail_signature"><div dir="ltr"><p><b><i>Tommaso Seita</i></b></p>
<p><font color="#674ea7">Co Founder &amp; CEO at Esempio</font></p><p>A: Corso Esempio, 1, 10100 Torino</p>
<p>Website: <a href="https://esempio.it/">esempio.it</a></p><img width="96" height="34" src="https://picsum.photos/96/34"></div></div>
<br><div class="gmail_quote"><div dir="ltr" class="gmail_attr">Il giorno lun 3 nov 2025, Giulia Neri ha scritto:</div>
<blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex">Ciao Tommaso, cosa vi serve per partire?</blockquote></div></div>`;
  const text = "Ciao Giulia, di seguito le cose di cui avremmo bisogno:\n\n* numero e data del preventivo firmato\n\n* elenco dei partecipanti\n\nGrazie!\n\n*Tommaso Seita*\n\nCo Founder & CEO at Esempio";
  const giulia = PEOPLE[4];
  threads.unshift({ id: "t9999", subject: "Documenti per partire con il corso", msgs: [
    { id: "m9999-0", date: now - 90 * 60_000, from: giulia, to: [ME], labels: ["INBOX"], html: null, text: "Ciao Tommaso, cosa vi serve per partire?\n\nGiulia", snippet: "Ciao Tommaso, cosa vi serve per partire?" },
    { id: "m9999-1", date: now - 30 * 60_000, from: ME, to: [giulia], labels: ["SENT", "INBOX"], html, text, snippet: "Ciao Giulia, di seguito le cose di cui avremmo bisogno" },
  ] });
}

// Una risposta da Outlook (11 novembre): la citazione non ha classi, solo l'intestazione "From: … Sent: …"
// sotto una riga sottile; tanti destinatari in Cc; un logo senza misure che non arriva mai (l'attesa
// della conversazione ha un tetto).
{
  const html = `<div class="WordSection1"><p class="MsoNormal">Buongiorno Marco, tutto bene?</p><p class="MsoNormal">&nbsp;</p>
<p class="MsoNormal">Ne ho parlato con la direzione: per ora preferiamo rimandare il progetto al prossimo trimestre. Se vuoi
ne parliamo a voce la settimana prossima.</p><p class="MsoNormal">&nbsp;</p>
<p class="MsoNormal">Grazie!</p><p class="MsoNormal">&nbsp;</p><p class="MsoNormal"><b>Davide Ferri</b><br>Responsabile acquisti<br><img src="https://10.255.255.1/logo-che-non-arriva.png" alt=""></p>
<p class="MsoNormal">&nbsp;</p><div><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0cm 0cm 0cm">
<p class="MsoNormal"><b><span lang="EN-US">From:</span></b><span lang="EN-US"> Marco Rossi &lt;marco@esempio.it&gt;<br><b>Sent:</b> Monday, November 9, 2026 7:22 PM<br>
<b>To:</b> Davide Ferri &lt;davide@fornitore.it&gt;<br><b>Cc:</b> Tommaso Seita &lt;tommaso@esempio.it&gt;<br><b>Subject:</b> Re: Presentazione</span></p></div></div>
<p class="MsoNormal">&nbsp;</p><p class="MsoNormal">Ciao Davide, ti mando la presentazione di cui parlavamo.</p>
<p class="MsoNormal">A presto,<br>Marco</p><p class="MsoNormal">Da: un altro messaggio ancora più vecchio</p></div>`;
  const davide = { name: "Davide Ferri (Fornitore)", email: "davide@fornitore.it" };
  const cc = ["Marco Rossi", "Elisa Fontana", "Pietro Sala", "Chiara Longo", "Irene Villa", "Sofia Marino", "Ufficio Acquisti Fornitore"]
    .map((name) => ({ name, email: `${name.split(" ")[0].toLowerCase()}@esempio.it` }));
  threads.unshift({ id: "t9998", subject: "Presentazione del corso per Fornitore", msgs: [
    { id: "m9998-0", date: now - 20 * 60_000, from: davide, to: [cc[0]], cc: [ME, ...cc.slice(1)], labels: ["INBOX", "UNREAD"], html,
      text: "Buongiorno Marco, tutto bene?\n\nNe ho parlato con la direzione.\n\nFrom: Marco Rossi\nSent: Monday\nSubject: Re: Presentazione\n\nCiao Davide",
      snippet: "Buongiorno Marco, tutto bene? Ne ho parlato con la direzione" },
  ] });
}

// Immagini incollate nel testo (12 novembre): `cid:`, una con le misure e una senza.
{
  const html = `<div dir="ltr"><p>Ciao Tommaso, ti giro due segnalazioni.</p>
<p><b>Luca Verdi, Torino</b>: questa è la sua schermata. Non riesce a prenotare il colloquio.</p>
<p><img src="cid:ii_schermata1" alt="Verdi.jpeg" width="562" height="320"></p>
<p><b>Sara Conti, Milano</b>: il sistema le propone orari stranissimi.</p>
<p><img src="cid:ii_schermata2" alt="Conti.jpeg"></p><p>Puoi scrivere a tutti e due?</p></div>`;
  const anna = PEOPLE[1];
  threads.unshift({ id: "t9996", subject: "Due segnalazioni sulle prenotazioni", msgs: [
    { id: "m9996-0", date: now - 5 * 60_000, from: anna, to: [ME], labels: ["INBOX"], html,
      text: "Ciao Tommaso, ti giro due segnalazioni.\n\nLuca Verdi, Torino: questa è la sua schermata.", snippet: "Ciao Tommaso, ti giro due segnalazioni",
      inline: [{ cid: "ii_schermata1", name: "Verdi.jpeg" }, { cid: "ii_schermata2", name: "Conti.jpeg" }] },
  ] });
}

// Un inoltro senza testo mio: senza citazioni non resta niente, la conversazione mostra l'inoltrato.
{
  const html = `<div dir="ltr"><br><div class="gmail_quote"><div dir="ltr" class="gmail_attr">---------- Forwarded message ---------<br>
Da: <b>Ufficio Fatture</b> &lt;fatture@fornitore.it&gt;<br>Date: lun 10 nov 2026<br>Subject: Fattura di ottobre<br></div><br>
<p>Buongiorno, in allegato la fattura di ottobre.</p><p>Cordiali saluti,<br>Ufficio Fatture<br><img src="https://picsum.photos/120/40" width="120" height="40"></p></div></div>`;
  const anna = PEOPLE[1];
  threads.unshift({ id: "t9997", subject: "Fwd: Fattura di ottobre", msgs: [
    { id: "m9997-0", date: now - 10 * 60_000, from: anna, to: [ME], labels: ["INBOX"], html,
      text: "---------- Forwarded message ---------\nBuongiorno, in allegato la fattura di ottobre.", snippet: "Forwarded message: Buongiorno, in allegato la fattura" },
  ] });
}

const labelsOf = (t: Th) => [...new Set(t.msgs.flatMap((m) => m.labels))];
const has = (t: Th, l: string) => t.msgs.some((m) => m.labels.includes(l));
const last = (t: Th) => {
  const visible = t.msgs.filter((m) => !m.labels.includes("DRAFT"));
  return visible[visible.length - 1] ?? t.msgs[t.msgs.length - 1];
};

// Configurazione delle sezioni, sintesi, posticipi e coda di invio finti.
const sectionCfg = new Map<string, { position: number; archived: boolean; show_as_tab: boolean; instructions: string | null }>();
let aiKeys = new Set<string>();
let aiConfig = { provider: "mistral", model: "mistral-small-2603" };
// ?primo-avvio: come al primo avvio, senza client Google né account.
const firstRun = new URLSearchParams(location.search).has("primo-avvio");
const slow = new URLSearchParams(location.search).has("lento");
// ?corpi-tardi: come i thread fuori dalla finestra di sincronizzazione, i corpi arrivano da Gmail dopo
// l'apertura (get_thread li mette in coda, poi threads_changed).
const lateBodies = new URLSearchParams(location.search).has("corpi-tardi");
const downloaded = new Set<string>();
let googleClient: { client_id: string; project_id: string | null } | null = firstRun ? null : { client_id: "123456-abc.apps.googleusercontent.com", project_id: "posta-di-tommaso" };
let loggedIn = !firstRun;
let myDomains: string[] | null = null;
const reminded = new Set<string>();
const snoozes = new Map<number, { thread: string; due: number; ops: number[] }>();
let nextJob = 1;
const outbox = new Map<number, { draft: any; at: number; state: string; timer?: ReturnType<typeof setTimeout> }>();
function sendLater(id: number, ms: number) {
  const o = outbox.get(id);
  if (!o) return;
  clearTimeout(o.timer);
  o.timer = setTimeout(() => {
    if (o.state !== "pending") return;
    o.state = "sent";
    emit("core", { type: "send_status", id, state: "sent", error: null });
  }, ms);
}
let nextSend = 1;


function summary(t: Th) {
  const ls = labelsOf(t);
  const senders: { name: string; email: string; unread: boolean }[] = [];
  for (const m of t.msgs) {
    const p = senders.find((s) => s.email === m.from.email);
    if (p) p.unread ||= m.labels.includes("UNREAD");
    else senders.push({ name: m.from.name, email: m.from.email, unread: m.labels.includes("UNREAD") });
  }
  return {
    id: t.id, subject: t.subject, snippet: last(t).snippet, last_date: last(t).date, participants: senders,
    message_count: t.msgs.length, unread: ls.includes("UNREAD"), starred: ls.includes("STARRED"),
    in_inbox: ls.includes("INBOX") && !ls.includes("TRASH"), important: false, has_attachments: t.msgs.some((m) => m.attach),
    has_draft: ls.includes("DRAFT"), bodies_complete: true, labels: ls,
    reminded: reminded.has(t.id),
    attachment_names: [...new Set(t.msgs.map((m) => m.attach).filter((a): a is string => !!a))].slice(0, 3),
    attachment_count: t.msgs.filter((m) => m.attach).length,
  };
}

// Inbox divisa, come nel nucleo ma semplificata: newsletter, no-reply e promozioni in Altro.
const overrides = new Map<string, string>();
const splitRules: { kind: string; pattern: string; split: string; created_at: number }[] = [];
function splitOf(t: Th): string {
  const o = overrides.get(t.id);
  if (o) return o;
  const sender = t.msgs.filter((m) => m.from.email !== ME.email).map((m) => m.from.email).pop() ?? "";
  const rule = splitRules.find((r) => (r.kind === "sender" && r.pattern === sender) || (r.kind === "domain" && sender.endsWith(`@${r.pattern}`)));
  if (rule) return rule.split;
  if (sender.endsWith("@esempio.it")) return "important";
  if (has(t, "CATEGORY_PROMOTIONS") || /^(news|noreply|no-reply)/.test(sender)) return "other";
  return Number(t.id.slice(1)) % 5 === 0 ? "other" : "important";
}

type View = { kind: "inbox" | "important" | "other" | "done" | "all" } | { kind: "label" | "section"; id: string } | { kind: "search"; q: string };
const inTabSection = (t: Th) => [...sectionCfg.entries()].some(([id, c]) => c.show_as_tab && !c.archived && has(t, id));
const fold = (s: string) => s.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
function inView(t: Th, v: View): boolean {
  const trash = has(t, "TRASH") || has(t, "SPAM");
  switch (v.kind) {
    case "inbox": return has(t, "INBOX") && !trash;
    case "important": return has(t, "INBOX") && !trash && splitOf(t) === "important" && !inTabSection(t);
    case "other": return has(t, "INBOX") && !trash && splitOf(t) === "other" && !inTabSection(t);
    case "section": return has(t, "INBOX") && !trash && has(t, v.id);
    case "done": return !has(t, "INBOX") && !trash && t.msgs.some((m) => !m.labels.includes("SENT") && !m.labels.includes("DRAFT"));
    case "all": return !trash;
    case "label": return has(t, v.id) && (v.id === "TRASH" || v.id === "SPAM" || !trash);
    case "search": {
      const hay = fold(`${t.subject} ${t.msgs.map((m) => `${m.from.name} ${m.from.email} ${m.text}`).join(" ")}`);
      return !trash && fold(v.q).split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
    }
  }
}
const sortKey = (t: Th) => (reminded.has(t.id) ? now + 1 : last(t).date);
const sorted = (v: View) => threads.filter((t) => inView(t, v)).sort((a, b) => sortKey(b) - sortKey(a));

function detail(id: string) {
  const t = threads.find((x) => x.id === id);
  if (!t) return null;
  return {
    thread: summary(t),
    messages: t.msgs.map((m) => ({
      id: m.id, date: m.date, from: { name: m.from.name, email: m.from.email }, to: m.to.map((p) => ({ name: p.name, email: p.email })),
      ...(lateBodies && !downloaded.has(t.id)
        ? { has_body: false, text_plain: null, html: null }
        : { has_body: true, text_plain: m.text, html: m.html }),
      cc: (m.cc ?? []).map((p) => ({ name: p.name, email: p.email })), subject: t.subject, snippet: m.snippet, label_ids: m.labels,
      attachments: [
        ...(m.attach ? [{ part_id: "1", filename: m.attach, mime_type: "application/pdf", size: 184_000, content_id: null, is_inline: false }] : []),
        // Come Outlook: immagini nel testo, ma dichiarate come allegati normali.
        ...(m.inline ?? []).map((x, i) => ({ part_id: `2.${i}`, filename: x.name, mime_type: "image/jpeg", size: 96_000, content_id: x.cid, is_inline: false })),
      ],
      trusted_sender: m.from.email.endsWith("@esempio.it") || !["news@prodotto.io", "noreply@banca-esempio.it"].includes(m.from.email),
      text_clean: m.text.split("\n\nIl giorno")[0].split("\n\nA presto,")[0].trim(),
      bulk: /^(news|noreply)/.test(m.from.email),
    })),
  };
}

// ───────── eventi ─────────
const callbacks = new Map<number, (e: unknown) => void>();
const listeners = new Map<string, Set<number>>();
let nextCb = 1;
function emit(event: string, payload: unknown) {
  for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
}
const changed = (ids: string[]) => setTimeout(() => emit("core", { type: "threads_changed", ids }), 30);

// ───────── azioni ─────────
let nextOp = 1;
const inverse = new Map<number, { ids: string[]; add: string[]; remove: string[] }>();
function modify(ids: string[], add: string[], remove: string[]) {
  for (const t of threads.filter((x) => ids.includes(x.id))) {
    for (const m of t.msgs) {
      m.labels = m.labels.filter((l) => !remove.includes(l));
      if (!m.labels.includes("SENT") || add.includes("STARRED")) for (const a of add) if (!m.labels.includes(a)) m.labels.push(a);
    }
  }
}
function delta(a: { type: string; label_id?: string }): [string[], string[]] {
  switch (a.type) {
    case "archive": return [[], ["INBOX"]];
    case "move_to_inbox": return [["INBOX"], []];
    case "mark_read": return [[], ["UNREAD"]];
    case "mark_unread": return [["UNREAD"], []];
    case "star": return [["STARRED"], []];
    case "unstar": return [[], ["STARRED"]];
    case "trash": return [["TRASH"], ["INBOX"]];
    case "untrash": return [["INBOX"], ["TRASH"]];
    case "add_label": return [[a.label_id!], []];
    default: return [[], [a.label_id!]];
  }
}

const settings = new Map<string, unknown>();
const bulks = new Map<number, string[]>();

const LABELS = [
  ["INBOX", "INBOX", "system"], ["SENT", "SENT", "system"], ["DRAFT", "DRAFT", "system"], ["STARRED", "STARRED", "system"],
  ["IMPORTANT", "IMPORTANT", "system"], ["SPAM", "SPAM", "system"], ["TRASH", "TRASH", "system"], ["UNREAD", "UNREAD", "system"],
  ["Label_1", "Clienti", "user"], ["Label_2", "Fatture", "user"],
].map(([id, name, kind]) => ({ id, name, kind, color_bg: null as string | null, color_text: null, label_list_visibility: null, message_list_visibility: null }));
LABELS[8].color_bg = "#f0b232";
LABELS[9].color_bg = "#5bb98c";

function sections() {
  const user = LABELS.filter((l) => l.kind === "user" && !l.name.toLowerCase().startsWith("dimail/"));
  user.forEach((l, i) => { if (!sectionCfg.has(l.id)) sectionCfg.set(l.id, { position: 100 + i, archived: false, show_as_tab: false, instructions: null }); });
  return user
    .map((l) => {
      const c = sectionCfg.get(l.id)!;
      const inbox = threads.filter((t) => has(t, l.id) && has(t, "INBOX") && !has(t, "TRASH"));
      return { label_id: l.id, name: l.name, color_bg: l.color_bg, ...c, inbox: inbox.length, unread: inbox.filter((t) => has(t, "UNREAD")).length };
    })
    .sort((a, b) => a.position - b.position);
}

function quote(t: Th, m: Msg, fwd: boolean) {
  const when = new Date(m.date).toLocaleString("it-IT", { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  return fwd
    ? `---------- Messaggio inoltrato ----------\nDa: ${m.from.name} <${m.from.email}>\nOggetto: ${t.subject}\n\n${m.text}`
    : `Il ${when}, ${m.from.name} <${m.from.email}> ha scritto:\n${m.text.split("\n").map((l) => `> ${l}`).join("\n")}`;
}

async function handle(cmd: string, a: Record<string, any>): Promise<unknown> {
  switch (cmd) {
    case "plugin:event|listen": {
      const set = listeners.get(a.event) ?? new Set();
      set.add(a.handler);
      listeners.set(a.event, set);
      return a.handler;
    }
    case "plugin:event|unlisten": listeners.get(a.event)?.delete(a.eventId); return null;
    case "plugin:opener|open_url": window.open(a.url, "_blank", "noopener"); return null;
    case "log_frontend": console.log(`[${a.level}] ${a.message}`); return null;
    case "app_info": return loggedIn
      ? { starting: false, account: { email: ME.email, name: ME.name, picture_url: null, history_id: 1, initial_sync_state: "done", last_sync_at: now }, needs_login: false, login_hint: ME.email, needs_client: !googleClient }
      : { starting: false, account: null, needs_login: true, login_hint: null, needs_client: !googleClient };
    case "login": await new Promise((r) => setTimeout(r, 1200)); loggedIn = true; return null;
    case "google_client": return googleClient;
    case "load_google_client": {
      await new Promise((r) => setTimeout(r, 500));
      if (a.path && !String(a.path).endsWith(".json")) throw "il file non è un JSON: scarica quello del client dalla console di Google Cloud";
      googleClient = { client_id: "987654-xyz.apps.googleusercontent.com", project_id: "posta-mia" };
      return googleClient;
    }
    // Link di configurazione: nel finto backend il link porta il client "posta-aziendale".
    case "client_setup_link": return "dimail://client?c=cG9zdGEtYXppZW5kYWxl";
    case "setup_link_info": {
      if (!String(a.link).startsWith("dimail://client?c=")) throw "non è un link di configurazione di diMail";
      const client_id = "555000-org.apps.googleusercontent.com";
      return { client_id, project_id: "posta-aziendale", same: googleClient?.client_id === client_id, replaces: !!googleClient && googleClient.client_id !== client_id };
    }
    case "apply_setup_link": {
      await new Promise((r) => setTimeout(r, 300));
      const changed = !!googleClient && googleClient.client_id !== "555000-org.apps.googleusercontent.com";
      googleClient = { client_id: "555000-org.apps.googleusercontent.com", project_id: "posta-aziendale" };
      if (changed) loggedIn = false;
      return googleClient;
    }
    case "plugin:deep-link|get_current": return null;
    case "my_domains": {
      const suggested = ME.email.split("@")[1];
      return { domains: myDomains ?? [suggested], custom: myDomains !== null, suggested };
    }
    case "set_my_domains": {
      const list: string[] = [];
      for (const d of a.domains as string[]) {
        const n = d.trim().replace(/^@/, "").toLowerCase();
        if (!n) continue;
        if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(n)) throw `«${d.trim()}» non è un dominio (es. azienda.it)`;
        if (!list.includes(n)) list.push(n);
      }
      myDomains = list;
      return { domains: list, custom: true, suggested: ME.email.split("@")[1] };
    }
    case "list_threads": return sorted(a.view).slice(a.offset, a.offset + a.limit).map(summary);
    case "count_threads": return sorted(a.view).length;
    case "get_thread": case "peek_thread":
      // ?lento: la lettura costa come nell'app vera (IPC, database, Gmail per i corpi mancanti).
      if (slow) await new Promise((r) => setTimeout(r, cmd === "get_thread" ? 400 : 120));
      if (lateBodies && cmd === "get_thread" && !downloaded.has(a.id)) {
        setTimeout(() => {
          downloaded.add(a.id);
          changed([a.id]);
        }, 900);
      }
      return detail(a.id);
    case "list_labels": return LABELS.map((l) => ({ ...l, unread_local: threads.filter((t) => has(t, l.id) && has(t, "UNREAD") && !has(t, "TRASH")).length }));
    case "sync_status": return { phase: "idle", initial_sync_state: "done", queued: 0, queued_background: 0, pending_actions: 0, threads_local: threads.length, inbox_local: sorted({ kind: "inbox" }).length, last_sync_at: now, last_error: null };
    case "sync_now": return null;
    case "act": {
      const [add, remove] = delta(a.action);
      modify(a.threadIds, add, remove);
      const op = nextOp++;
      inverse.set(op, { ids: a.threadIds, add: remove, remove: add });
      changed(a.threadIds);
      return [op];
    }
    case "undo": {
      for (const op of a.opIds) {
        const inv = inverse.get(op);
        if (inv) modify(inv.ids, inv.add, inv.remove);
        changed(inv?.ids ?? []);
      }
      return a.opIds;
    }
    case "contact_card": {
      const p = PEOPLE.find((x) => x.email === a.email);
      const mine = threads.filter((t) => t.msgs.some((m) => m.from.email === a.email));
      const recent = mine.sort((x, y) => last(y).date - last(x).date).slice(0, 5).map(summary);
      const dates = mine.flatMap((t) => t.msgs.filter((m) => m.from.email === a.email).map((m) => m.date));
      const domain = a.email.split("@")[1];
      return {
        email: a.email, name: p?.name ?? null, role: p?.role ?? null, phone: p?.phone ?? null,
        company: ["gmail.com"].includes(domain) ? null : domain.split(".")[0].replace(/-/g, " ").replace(/\b\w/g, (c: string) => c.toUpperCase()),
        received_count: dates.length, sent_count: Math.floor(dates.length / 2),
        first_date: dates.length ? Math.min(...dates) : null, last_date: dates.length ? Math.max(...dates) : null, recent,
      };
    }
    case "done_today": return 3;
    case "move_split": {
      for (const id of a.threadIds) {
        const t = threads.find((x) => x.id === id);
        const sender = t?.msgs.filter((m) => m.from.email !== ME.email).map((m) => m.from.email).pop();
        if (a.remember === "thread" || !sender) overrides.set(id, a.split);
        else {
          overrides.delete(id);
          const pattern = a.remember === "domain" ? sender.split("@")[1] : sender;
          const i = splitRules.findIndex((r) => r.kind === a.remember && r.pattern === pattern);
          if (i >= 0) splitRules.splice(i, 1);
          splitRules.unshift({ kind: a.remember, pattern, split: a.split, created_at: Date.now() });
        }
      }
      changed(a.remember === "thread" ? a.threadIds : []);
      return null;
    }
    case "split_rules": return splitRules;
    case "remove_split_rule": {
      const i = splitRules.findIndex((r) => r.kind === a.kind && r.pattern === a.pattern);
      if (i >= 0) splitRules.splice(i, 1);
      changed([]);
      return null;
    }
    case "get_setting": return settings.get(a.key) ?? null;
    case "set_setting": settings.set(a.key, a.value); return null;
    case "bulk_preview": {
      const n = sorted({ kind: "inbox" }).filter((t) => last(t).date < a.cutoff).length;
      return { cutoff: a.cutoff, query: "in:inbox before:…", local: n, estimate: n };
    }
    case "bulk_done": {
      const ids = sorted({ kind: "inbox" }).filter((t) => last(t).date < a.cutoff).map((t) => t.id);
      modify(ids, [], ["INBOX"]);
      const op = nextOp++;
      bulks.set(op, ids);
      changed([]);
      const total = ids.length * 2;
      [0.25, 0.5, 0.75, 1].forEach((f, i) =>
        setTimeout(() => emit("core", { type: "bulk_status", op_id: op, kind: "done", state: f < 1 ? "applying" : "done", done: Math.round(total * f), total }), 700 * (i + 1)));
      return op;
    }
    case "snooze": {
      const [add, remove] = [[], ["INBOX"]];
      modify(a.threadIds, add, remove);
      const op = nextOp++;
      inverse.set(op, { ids: a.threadIds, add: remove, remove: add });
      const jobs = a.threadIds.map((id: string) => {
        const j = nextJob++;
        snoozes.set(j, { thread: id, due: a.dueAt, ops: [op] });
        // Nel finto il posticipo dura 20 secondi, per vederlo tornare.
        setTimeout(() => {
          if (!snoozes.has(j)) return;
          snoozes.delete(j);
          modify([id], ["INBOX"], []);
          reminded.add(id);
          changed([id]);
        }, 20_000);
        return j;
      });
      changed(a.threadIds);
      return { ops: [op], jobs };
    }
    case "unsnooze": {
      for (const j of a.handle.jobs) snoozes.delete(j);
      return handle("undo", { opIds: a.handle.ops }).then(() => null);
    }
    case "list_snoozed": return [...snoozes.entries()].map(([job_id, s]) => ({ job_id, thread_id: s.thread, due_at: s.due }));
    case "clear_reminder": reminded.delete(a.threadId); return null;
    case "reply_draft": {
      const t = threads.find((x) => x.id === a.threadId)!;
      const m = last(t);
      const fwd = a.mode === "forward";
      const to = fwd ? [] : [m.from.email === ME.email ? m.to[0] : m.from].map((p) => ({ name: p.name, email: p.email }));
      const cc = a.mode === "reply_all" ? [{ name: "Luca Bianchi", email: "luca@esempio.it" }] : [];
      const subject = fwd ? `Fwd: ${t.subject}` : /^re:/i.test(t.subject) ? t.subject : `Re: ${t.subject}`;
      const attachments = fwd && m.attach ? [{ name: m.attach, size: 182_400, mime_type: "application/pdf", path: null, message_id: m.id, part_id: "1" }] : [];
      return { to, cc, bcc: [], subject, body: "", thread_id: fwd ? null : t.id, in_reply_to: fwd ? null : `<${m.id}@x>`, references: null, quoted: quote(t, m, fwd), attachments };
    }
    case "send_draft": {
      const id = nextSend++;
      const at = Date.now() + a.delayMs;
      outbox.set(id, { draft: a.draft, at, state: "pending" });
      sendLater(id, a.delayMs + 400);
      return { id, send_at: at };
    }
    case "send_now": {
      const o = outbox.get(a.id);
      if (!o || o.state !== "pending") return false;
      sendLater(a.id, 600);
      return true;
    }
    case "cancel_send": {
      const o = outbox.get(a.id);
      if (!o || o.state !== "pending") return null;
      o.state = "cancelled";
      return o.draft;
    }
    case "suggest_contacts": {
      const q = fold(a.q);
      return PEOPLE.filter((p) => fold(`${p.name} ${p.email}`).includes(q)).slice(0, 8).map((p) => ({ name: p.name, email: p.email }));
    }
    case "ai_status": return { has_key: aiKeys.has(aiConfig.provider), provider: aiConfig.provider, model: aiConfig.model, keys: [...aiKeys], tokens_today: 41_200, daily_cap: 2_000_000 };
    case "ai_set_key": {
      await new Promise((r) => setTimeout(r, 600));
      if (String(a.key).length < 8) throw `${a.provider === "openai" ? "OpenAI" : "Mistral"} non accetta questa chiave`;
      aiKeys.add(a.provider);
      return null;
    }
    case "ai_clear_key": aiKeys.delete(a.provider); return null;
    case "ai_models": return [
      { provider: "mistral", id: "mistral-small-2603", name: "Mistral Small 4", recommended: true },
      { provider: "mistral", id: "mistral-medium-2604", name: "Mistral Medium 3.5", recommended: false },
      { provider: "mistral", id: "mistral-large-4-0", name: "Mistral Large 4", recommended: false },
      { provider: "openai", id: "gpt-5.4-mini", name: "GPT-5.4 mini", recommended: true },
      { provider: "openai", id: "gpt-6-luna", name: "GPT-6 Luna", recommended: false },
      { provider: "openai", id: "gpt-6.1-terra", name: "GPT-6.1 Terra", recommended: false },
      { provider: "openai", id: "gpt-6.1-sol", name: "GPT-6.1 Sol", recommended: false },
    ];
    case "ai_configure": aiConfig = { provider: a.provider, model: a.model }; return null;
    case "ai_check": {
      await new Promise((r) => setTimeout(r, 700));
      if (!aiKeys.has(aiConfig.provider)) throw `manca la chiave di ${aiConfig.provider === "openai" ? "OpenAI" : "Mistral"}`;
      if (aiConfig.model.includes("x")) throw `Mistral ha risposto 400 con ${aiConfig.model}: invalid model`;
      return 640;
    }
    case "open_attachment": await new Promise((r) => setTimeout(r, 700)); console.info("apro l'allegato", a.messageId, a.partId); return null;
    case "save_attachment": await new Promise((r) => setTimeout(r, 500)); return "/Users/tommaso/Downloads/preventivo.pdf";
    case "inline_image": {
      await new Promise((r) => setTimeout(r, slow ? 900 : 120));
      const second = String(a.partId).endsWith(".1");
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${second ? 900 : 1124}" height="${second ? 420 : 640}"><rect width="100%" height="100%" fill="${second ? "#dbe7ff" : "#ffe9c7"}"/><text x="40" y="90" font-size="48" font-family="sans-serif" fill="#333">${second ? "Conti" : "Verdi"}: schermata</text></svg>`;
      return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
    }
    case "reveal_download": return null;
    case "get_signature":
      return { html: '<div dir="ltr"><b>Tommaso Seita</b><div>CEO &amp; Co-founder · Esempio</div><div><a href="https://esempio.it">esempio.it</a> · +39 333 123 4567</div></div>', text: "Tommaso Seita\nCEO & Co-founder · Esempio" };
    case "pick_attachments":
      await new Promise((r) => setTimeout(r, 300));
      return [{ name: "Investor Deck Esempio - novembre 2026.pdf", size: 8_412_000, mime_type: "application/pdf", path: "/Users/tommaso/Desktop/Investor Deck Esempio - novembre 2026.pdf" }];
    case "file_info":
      return (a.paths as string[]).map((p) => ({ name: p.split("/").pop(), size: 240_000, mime_type: "application/octet-stream", path: p }));
    case "ai_write": {
      if (!aiKeys.has(aiConfig.provider)) throw "manca la chiave dell'AI";
      await new Promise((r) => setTimeout(r, 1200));
      const r = a.req;
      const t = r.thread_id ? threads.find((x) => x.id === r.thread_id) : null;
      const who = (r.to[0]?.name ?? (t ? last(t).from.name : "")).split(" ")[0] || "";
      const notes = String(r.notes ?? "").trim();
      const core = notes
        ? notes.charAt(0).toUpperCase() + notes.slice(1).replace(/\s+/g, " ").replace(/([^.!?])$/, "$1.")
        : "grazie per il messaggio. Ci guardo oggi e ti rispondo entro domani con tutti i dettagli.";
      return `Ciao ${who},\n\n${notes ? `ti scrivo per aggiornarti: ${core.charAt(0).toLowerCase()}${core.slice(1)}` : core}\n\nFammi sapere se ti serve altro.\n\nUn saluto,\nTommaso`;
    }
    case "list_sections": return sections();
    case "create_section": {
      const id = `Label_${LABELS.length + 1}`;
      LABELS.push({ id, name: a.name, kind: "user", color_bg: null, color_text: null, label_list_visibility: null, message_list_visibility: null });
      for (const c of sectionCfg.values()) c.position += 1;
      sectionCfg.set(id, { position: 0, archived: false, show_as_tab: a.showAsTab, instructions: a.instructions });
      if (a.instructions && aiKeys.has(aiConfig.provider)) {
        // L'AI "smista": un thread su nove in inbox finisce nella sezione.
        setTimeout(() => {
          const hit = sorted({ kind: "inbox" }).filter((_, i) => i % 9 === 2).slice(0, 14).map((t) => t.id);
          modify(hit, [id], []);
          changed([]);
          emit("core", { type: "labels_changed" });
        }, 1500);
      }
      emit("core", { type: "labels_changed" });
      return sections().find((s) => s.label_id === id);
    }
    case "rename_section": {
      const l = LABELS.find((x) => x.id === a.labelId);
      if (l) l.name = a.name;
      emit("core", { type: "labels_changed" });
      return null;
    }
    case "configure_section": {
      sections();
      const c = sectionCfg.get(a.labelId)!;
      c.show_as_tab = a.showAsTab;
      c.instructions = a.instructions;
      changed([]);
      return null;
    }
    case "archive_section": {
      sections();
      sectionCfg.get(a.labelId)!.archived = a.archived;
      changed([]);
      return null;
    }
    case "reorder_sections": {
      sections();
      (a.ordered as string[]).forEach((id, i) => { const c = sectionCfg.get(id); if (c) c.position = i; });
      return null;
    }
    case "bulk_undo": {
      modify(bulks.get(a.opId) ?? [], ["INBOX"], []);
      changed([]);
      return nextOp++;
    }
    default:
      throw new Error(`comando finto non previsto: ${cmd}`);
  }
}

(window as any).__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args: Record<string, unknown> = {}) =>
    new Promise((ok, ko) => setTimeout(() => handle(cmd, args).then(ok, ko), 4)),
  transformCallback: (cb: (e: unknown) => void) => {
    const id = nextCb++;
    callbacks.set(id, cb);
    return id;
  },
  unregisterCallback: (id: number) => callbacks.delete(id),
  convertFileSrc: (p: string) => p,
  metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
};
(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
// Per provare a mano nel browser: __dimailMock.emit("tauri://drag-drop", { paths: [...], position: { x: 0, y: 0 } }).
(window as any).__dimailMock = { emit };

console.info("diMail: backend finto per lo sviluppo nel browser (dati inventati).");

export {};
