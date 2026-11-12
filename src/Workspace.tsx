// L'interfaccia di lavoro (specifica UX, aspetto rivisto l'8 novembre sul riferimento di Tommaso):
// barra a sinistra, schede in alto, thread come schede, conversazione come una chat. L'inbox è una lista
// di cose da fare, tutto si fa da tastiera, i comandi stanno in un registro unico (sez. 7 della spec).
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { Login } from "./App";
import {
  api, emptyDraft, onCoreEvent, type AccountRow, type Action, type Addr, type AiStatus, type BulkStatus, type ContactCard, type Draft,
  type LabelRow, type Remember, type ReplyMode, type Section, type Split, type SplitRule, type SyncStatus, type ThreadDetail,
  type ThreadSummary, type View,
} from "./api";
import { BulkDialog } from "./BulkDialog";
import { KeyRouter, available, type Command, type Context } from "./commands";
import { Composer, type ComposeMode } from "./Composer";
import { ContactBar } from "./ContactBar";
import { describe, report } from "./diagnostics";
import { useFileDrop } from "./fileDrop";
import { avatarColor, clock, count, initials, personOf, startOfToday } from "./format";
import * as I from "./icons";
import { Help, Palette, type PaletteItem } from "./Palette";
import { SectionDialog } from "./SectionDialog";
import { SettingsDialog, type SettingsTab } from "./SettingsDialog";
import { SnoozePicker } from "./SnoozePicker";
import { ThreadList } from "./ThreadList";
import { ThreadView } from "./ThreadView";
import { useUpdate } from "./updates";

type Toast = {
  id: number;
  text: string;
  undo?: boolean;
  error?: boolean;
  action?: { label: string; run: () => void };
  /** Millisecondi prima che sparisca da solo (0 = resta). */
  ttl?: number;
};
type Overlay = null | "palette" | "labels" | "help" | "bulk" | "bulk-offer" | "rules" | "remember" | "snooze" | "section" | "ai-key" | "settings";
type Compose = { draft: Draft; mode: ComposeMode; inline: boolean; key: number };

/** Il tempo per annullare un invio (30 secondi dal 9 novembre, sera). */
const SEND_DELAY = 30_000;

/** Un'email in coda di invio, nel popup in basso a sinistra. */
type Sending = { id: number; at: number; state: "pending" | "now" | "retrying" | "sent" };
// Su Gmail; fino al 9 novembre 2026 era "Dimail/Posticipati", quindi senza distinguere le maiuscole.
const SNOOZE_LABEL = "dimail/posticipati";
const isSnoozeLabel = (name?: string | null) => name?.toLowerCase() === SNOOZE_LABEL;

/** Dopo uno spostamento tra le schede: cosa ricordare (sez. 8 della spec). */
type Moved = { ids: string[]; to: Split; sender: string | null };

const PAGE = 300;
const DAY = 86_400_000;
const PRELOAD = 10;
const CACHE_MAX = 200;

const viewKey = (v: View) =>
  v.kind === "label" ? `label:${v.id}` : v.kind === "section" ? `section:${v.id}` : v.kind === "search" ? `search:${v.q}` : v.kind;

/** Dove si è: per Indietro e Avanti. */
type Loc = { view: View; openId: string | null };
/** Importanti e Altro: le due schede dell'inbox divisa (sez. 8). */
const isSplit = (v: View) => v.kind === "important" || v.kind === "other";
/** Una scheda in alto: Importanti, Altro o una sezione mostrata come scheda. */
const isTab = (v: View) => isSplit(v) || v.kind === "section";
/** Viste in cui Fatto e Posticipa tolgono il thread dalla lista. */
const isInboxish = (v: View) => v.kind === "inbox" || isTab(v);

const SYSTEM_NAMES: Record<string, string> = {
  INBOX: "Posta in arrivo", SENT: "Inviati", DRAFT: "Bozze", STARRED: "Speciali", IMPORTANT: "Importanti",
  SPAM: "Spam", TRASH: "Cestino", UNREAD: "Da leggere", CATEGORY_PERSONAL: "Personale", CATEGORY_SOCIAL: "Social",
  CATEGORY_PROMOTIONS: "Promozioni", CATEGORY_UPDATES: "Aggiornamenti", CATEGORY_FORUMS: "Forum",
};

function useLatest<T>(v: T) {
  const r = useRef(v);
  r.current = v;
  return r;
}

let toastSeq = 0;
let composeSeq = 0;

export function Workspace({ account, needsLogin, onRelogin }: { account: AccountRow; needsLogin: boolean; onRelogin: () => void }) {
  const me = account.email;
  const [view, setView] = useState<View>({ kind: "important" });
  const [tabCounts, setTabCounts] = useState<Record<string, number>>({});
  const [moved, setMoved] = useState<Moved | null>(null);
  const [rules, setRules] = useState<SplitRule[]>([]);
  const [rows, setRows] = useState<ThreadSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [cursor, setCursor] = useState(0);
  const [openId, setOpenId] = useState<string | null>(null);
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [labels, setLabels] = useState<LabelRow[]>([]);
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [bulk, setBulk] = useState<BulkStatus | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [sends, setSends] = useState<Sending[]>([]);
  const sendsRef = useRef(sends);
  sendsRef.current = sends;
  const [overlay, setOverlay] = useState<Overlay>(null);
  // La barra del contatto è aperta a ogni avvio (11 novembre); I la chiude fino alla prossima volta.
  const [contactOn, setContactOn] = useState(true);
  const [railOn, setRailOn] = useState(true);
  const [query, setQuery] = useState("");
  const [authLost, setAuthLost] = useState(needsLogin);
  const [sections, setSections] = useState<Section[]>([]);
  const [ai, setAi] = useState<AiStatus | null>(null);
  const [compose, setCompose] = useState<Compose | null>(null);
  const [snoozeIds, setSnoozeIds] = useState<string[]>([]);
  const [sectionEdit, setSectionEdit] = useState<{ section: Section | null; tab: boolean } | null>(null);
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [menu, setMenu] = useState<{ id: string; top: number; left: number } | null>(null);
  const [drag, setDrag] = useState<{ id: string; at: number | null } | null>(null);
  const [doneToday, setDoneToday] = useState(0);
  const [, rerender] = useReducer((x: number) => x + 1, 0);

  const details = useRef(new Map<string, ThreadDetail>());
  const undoStack = useRef<{ label: string; run: () => Promise<unknown> }[]>([]);
  const router = useRef(new KeyRouter());
  const loadingMore = useRef(false);
  const recentThreads = useRef<string[]>([]);
  const searchRef = useRef<HTMLInputElement>(null);
  const beforeSearch = useRef<View | null>(null);
  const hist = useRef<{ back: Loc[]; fwd: Loc[] }>({ back: [], fwd: [] });
  const pendingOpen = useRef<string | null>(null);
  const budgetNoted = useRef(false);
  const S = useLatest({ view, rows, total, cursor, openId, picked, labels, contactOn, railOn, overlay, sections, ai, compose });

  // ───────── messaggi brevi ─────────
  const dismissToast = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  const toast = useCallback((text: string, opts: Omit<Toast, "id" | "text"> = {}) => {
    const id = ++toastSeq;
    setToasts((t) => [...t.slice(-2), { id, text, ...opts }]);
    const ttl = opts.ttl ?? (opts.error ? 0 : 6000);
    if (ttl > 0) setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), ttl);
    return id;
  }, []);

  // ───────── dati ─────────
  const reload = useCallback(async (reset = false) => {
    const v = S.current.view;
    const want = reset ? PAGE : Math.min(Math.max(S.current.rows.length, PAGE), 3000);
    const tabSecs = S.current.sections.filter((s) => s.show_as_tab && !s.archived);
    try {
      const [list, n, important, other, ...secs] = await Promise.all([
        api.listThreads(v, want, 0),
        api.countThreads(v),
        api.countThreads({ kind: "important" }),
        api.countThreads({ kind: "other" }),
        ...tabSecs.map((s) => api.countThreads({ kind: "section", id: s.label_id })),
      ]);
      const counts: Record<string, number> = { important, other };
      tabSecs.forEach((s, i) => (counts[`section:${s.label_id}`] = secs[i]));
      setTabCounts(counts);
      if (viewKey(S.current.view) !== viewKey(v)) return;
      const prevId = reset ? null : S.current.rows[S.current.cursor]?.id;
      setRows(list);
      setTotal(n);
      setLoaded(true);
      if (prevId) {
        const i = list.findIndex((t) => t.id === prevId);
        setCursor((c) => (i >= 0 ? i : Math.min(c, Math.max(0, list.length - 1))));
      }
    } catch (e) {
      report("error", `lista: ${describe(e)}`);
    }
  }, [S]);

  const loadMore = useCallback(async () => {
    const { view: v, rows: have, total: n } = S.current;
    if (loadingMore.current || have.length >= n) return;
    loadingMore.current = true;
    try {
      const more = await api.listThreads(v, PAGE, have.length);
      if (viewKey(S.current.view) !== viewKey(v)) return;
      setRows((cur) => {
        if (cur.length !== have.length) return cur;
        const ids = new Set(cur.map((t) => t.id));
        return [...cur, ...more.filter((t) => !ids.has(t.id))];
      });
    } catch (e) {
      report("error", `altri thread: ${describe(e)}`);
    } finally {
      loadingMore.current = false;
    }
  }, [S]);

  const loadSide = useCallback(() => {
    api.listLabels().then(setLabels).catch(() => {});
    api.doneToday(startOfToday()).then(setDoneToday).catch(() => {});
  }, []);

  const loadSections = useCallback(() => api.listSections().then(setSections).catch(() => {}), []);

  // Thread con un caricamento completo (all'apertura) in corso: una lettura locale partita nel
  // frattempo può essere più vecchia, e non lo sovrascrive.
  const opening = useRef(new Map<string, number>());

  const fetchDetail = useCallback(async (id: string, full: boolean) => {
    if (full) opening.current.set(id, (opening.current.get(id) ?? 0) + 1);
    try {
      // Aprire chiede i corpi mancanti a Gmail; precaricare legge solo il locale.
      const d = full ? await api.getThread(id) : await api.peekThread(id);
      if (!d) return;
      if (!full && opening.current.get(id)) return;
      const cache = details.current;
      cache.delete(id);
      cache.set(id, d);
      while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value as string);
      rerender();
    } catch (e) {
      report("error", `thread: ${describe(e)}`);
    } finally {
      if (full) {
        const left = (opening.current.get(id) ?? 1) - 1;
        if (left > 0) opening.current.set(id, left);
        else opening.current.delete(id);
      }
    }
  }, []);

  // Cambio di vista: si riparte dall'inizio.
  const vk = viewKey(view);
  useEffect(() => {
    setRows([]);
    setTotal(0);
    setLoaded(false);
    setCursor(0);
    // Indietro e Avanti possono riportare a un thread aperto in un'altra vista.
    setOpenId(pendingOpen.current);
    pendingOpen.current = null;
    setPicked(new Set());
    reload(true);
  }, [vk, reload]);

  // Ricerca mentre si scrive (FTS5 per prefisso; gli operatori di Gmail arrivano con M3).
  useEffect(() => {
    const q = query.trim();
    const t = setTimeout(() => {
      const cur = S.current.view;
      if (q) {
        if (cur.kind !== "search") beforeSearch.current = cur;
        if (cur.kind !== "search" || cur.q !== q) setView({ kind: "search", q });
      } else if (cur.kind === "search") {
        setView(beforeSearch.current ?? { kind: "important" });
      }
    }, 180);
    return () => clearTimeout(t);
  }, [query, S]);

  // Il tema del Mac può cambiare da solo (giorno e notte): l'icona in alto lo segue.
  useEffect(() => {
    const mq = matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", rerender);
    return () => mq.removeEventListener("change", rerender);
  }, []);

  useEffect(() => {
    loadSide();
    loadSections();
    api.aiStatus().then(setAi).catch(() => {});
    api.syncStatus().then(setStatus).catch(() => {});
    // Preferenze dell'interfaccia (nel database dell'account).
    api.getSetting<string>("ui.theme").then((t) => applyTheme(t ?? "system")).catch(() => {});
    api.getSetting<boolean>("ui.rail").then((v) => v !== null && setRailOn(v)).catch(() => {});
  }, [loadSide, loadSections]);

  // Le schede delle sezioni cambiano: si ricontano.
  const tabKey = sections.filter((s) => s.show_as_tab && !s.archived).map((s) => s.label_id).join(",");
  useEffect(() => {
    if (tabKey) reload();
  }, [tabKey, reload]);

  // Eventi del nucleo: a raffica durante la sync, si rilegge al massimo ogni 150 ms.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    let touched = new Set<string>();
    let all = false;
    const flush = () => {
      timer = null;
      reload();
      loadSide();
      // Il thread aperto resta in cache finché non arriva la versione nuova: toglierlo prima faceva
      // sparire e ricomparire la conversazione (aprendo un thread non letto, due volte: per il
      // "letto" locale e quando Gmail lo conferma).
      const open = S.current.openId;
      const shown = open ? details.current.get(open) : undefined;
      if (all) details.current.clear();
      else touched.forEach((id) => details.current.delete(id));
      if (open && shown) details.current.set(open, shown);
      if (open && (all || touched.has(open))) fetchDetail(open, false);
      touched = new Set();
      all = false;
    };
    const un = onCoreEvent((e) => {
      switch (e.type) {
        case "threads_changed":
          if (e.ids.length === 0) all = true;
          e.ids.forEach((id) => touched.add(id));
          if (!timer) timer = setTimeout(flush, 150);
          break;
        case "labels_changed":
          api.listLabels().then(setLabels).catch(() => {});
          loadSections();
          break;
        case "ai_status":
          if (e.state === "budget" && !budgetNoted.current) {
            budgetNoted.current = true;
            toast("Raggiunto il tetto giornaliero dell'AI: riprende domani.");
          }
          break;
        case "send_status":
          if (e.state === "failed") {
            setSends((ss) => ss.filter((x) => x.id !== e.id));
            toast(`Non inviata: ${e.error ?? "errore"}. L'email è rimasta nella coda di invio.`, { error: true });
          } else {
            const state = e.state === "sent" ? "sent" : "retrying";
            setSends((ss) => ss.map((x) => (x.id === e.id ? { ...x, state } : x)));
            if (state === "sent") setTimeout(() => setSends((ss) => ss.filter((x) => x.id !== e.id)), 2500);
          }
          break;
        case "sync_status":
          setStatus(e);
          setAuthLost(e.phase === "auth_required");
          break;
        case "action_failed":
          toast(`Gmail ha rifiutato l'azione: ${e.message}`, { error: true });
          break;
        case "auth_required":
          setAuthLost(true);
          break;
        case "bulk_status":
          setBulk(e);
          if (e.state === "done" && e.kind === "done") toast(`Sgombero completato anche su Gmail (${count(e.done)} messaggi).`);
          if (e.state === "failed") toast("Lo sgombero su Gmail non è riuscito: i dettagli sono nel log.", { error: true });
          break;
      }
    });
    return () => {
      un.then((f) => f());
      if (timer) clearTimeout(timer);
    };
  }, [reload, loadSide, loadSections, fetchDetail, toast, S]);

  // Thread aperto: corpi mancanti chiesti a Gmail, segnato come letto, promemoria visto.
  useEffect(() => {
    if (!openId) return;
    fetchDetail(openId, true);
    const t = S.current.rows.find((r) => r.id === openId) ?? details.current.get(openId)?.thread;
    if (t?.unread) {
      setRows((rs) => rs.map((r) => (r.id === openId ? { ...r, unread: false } : r)));
      api.act([openId], { type: "mark_read" }).catch(() => {});
    }
    if (t?.reminded) {
      setRows((rs) => rs.map((r) => (r.id === openId ? { ...r, reminded: false } : r)));
      api.clearReminder(openId).catch(() => {});
    }
  }, [openId, fetchDetail, S]);

  // Aggiornamento scaricato: si riavvia solo su richiesta, e mai con un'email aperta o in partenza
  // (la bozza nel composer vive solo in memoria).
  const update = useUpdate();
  const restartForUpdate = useCallback(() => {
    if (S.current.compose) return toast("Hai un'email aperta: inviala o chiudila, poi riavvia.");
    if (sendsRef.current.some((x) => x.state === "pending" || x.state === "now")) return toast("C'è un'email in partenza: riavvia appena è inviata.");
    update.restart().catch((e) => toast(`Riavvio non riuscito: ${describe(e).split("\n")[0]}. Chiudi e riapri diMail.`, { error: true }));
  }, [S, toast, update]);

  const aiOn = !!ai?.has_key;

  /** ⌘J nel composer. Senza chiave apre la finestra per inserirla. */
  const aiWrite = useCallback(async (d: Draft, mode: ComposeMode): Promise<string> => {
    if (!S.current.ai?.has_key) {
      setOverlay("ai-key");
      throw new Error("Serve la chiave dell'AI");
    }
    return api.aiWrite({ mode, thread_id: d.thread_id, to: d.to, subject: d.subject, notes: d.body });
  }, [S]);

  // Precarica in memoria il thread selezionato e i 10 dopo (sez. 6), senza consumare quota.
  useEffect(() => {
    let alive = true;
    const ids = rows.slice(cursor, cursor + PRELOAD + 1).map((t) => t.id).filter((id) => !details.current.has(id));
    (async () => {
      for (const id of ids) {
        if (!alive) return;
        const d = await api.peekThread(id).catch(() => null);
        if (d && alive) details.current.set(id, d);
      }
    })();
    return () => {
      alive = false;
    };
  }, [cursor, rows]);

  // Proposta di sgombero, una volta sola, al primo avvio (specifica UX, decisione 3).
  const offered = useRef(false);
  useEffect(() => {
    if (!loaded || !isSplit(view) || offered.current) return;
    offered.current = true;
    (async () => {
      const done = await api.getSetting<boolean>("ui.bulk_offered").catch(() => true);
      if (done) return;
      const p = await api.bulkPreview(Date.now() - 30 * DAY).catch(() => null);
      if (!p) return;
      if (Math.max(p.local, p.estimate ?? 0) >= 1000) setOverlay("bulk-offer");
      else api.setSetting("ui.bulk_offered", true).catch(() => {});
    })();
  }, [loaded, view]);

  // ───────── azioni ─────────
  const currentThread = useCallback((): ThreadSummary | undefined => {
    const s = S.current;
    if (s.openId) return s.rows.find((r) => r.id === s.openId) ?? details.current.get(s.openId)?.thread;
    return s.rows[s.cursor];
  }, [S]);

  const targets = useCallback((): string[] => {
    const s = S.current;
    if (!s.openId && s.picked.size) return [...s.picked];
    const c = currentThread();
    return c ? [c.id] : [];
  }, [S, currentThread]);

  /** Toglie righe subito e porta avanti selezione e thread aperto (avanzamento automatico). */
  const removeRows = useCallback((ids: string[]) => {
    const s = S.current;
    const gone = new Set(ids);
    const anchor = s.openId ? s.rows.findIndex((r) => r.id === s.openId) : s.cursor;
    const next = s.rows.filter((r) => !gone.has(r.id));
    const before = anchor > 0 ? s.rows.slice(0, anchor).filter((r) => gone.has(r.id)).length : 0;
    const c = Math.max(0, Math.min(Math.max(anchor, 0) - before, next.length - 1));
    setRows(next);
    setTotal((t) => Math.max(0, t - (s.rows.length - next.length)));
    setCursor(c);
    setPicked(new Set());
    if (s.openId && gone.has(s.openId)) setOpenId(next[c]?.id ?? null);
  }, [S]);

  const patchRows = useCallback((ids: string[], patch: Partial<ThreadSummary>) => {
    const set = new Set(ids);
    setRows((rs) => rs.map((r) => (set.has(r.id) ? { ...r, ...patch } : r)));
  }, []);

  const act = useCallback(
    async (action: Action, label: string, opts: { removes?: boolean; patch?: Partial<ThreadSummary>; ids?: string[] } = {}) => {
      const ids = opts.ids ?? targets();
      if (!ids.length) return;
      if (opts.removes) removeRows(ids);
      else if (opts.patch) patchRows(ids, opts.patch);
      if (!opts.removes) setPicked(new Set());
      try {
        const ops = await api.act(ids, action);
        undoStack.current.push({ label, run: () => api.undo(ops) });
        toast(ids.length > 1 ? `${label}: ${count(ids.length)} thread` : label, { undo: true });
        if (action.type === "archive") setDoneToday((d) => d + ids.length);
      } catch (e) {
        toast(`Non riuscito: ${String(e)}`, { error: true });
        reload();
      }
    },
    [targets, removeRows, patchRows, toast, reload],
  );

  const undoLast = useCallback(async () => {
    const u = undoStack.current.pop();
    if (!u) return toast("Niente da annullare");
    try {
      await u.run();
      toast(`Annullato: ${u.label.toLowerCase()}`);
      reload();
    } catch (e) {
      toast(`Annulla non riuscito: ${String(e)}`, { error: true });
    }
  }, [toast, reload]);

  const move = useCallback((delta: number, extend = false) => {
    const s = S.current;
    if (!s.rows.length) return;
    const from = s.openId ? Math.max(0, s.rows.findIndex((r) => r.id === s.openId)) : s.cursor;
    const to = Math.max(0, Math.min(s.rows.length - 1, from + delta));
    if (extend) {
      setPicked((p) => new Set([...p, s.rows[from].id, s.rows[to].id]));
    }
    setCursor(to);
    if (s.openId) setOpenId(s.rows[to].id);
  }, [S]);

  // ───────── Indietro e Avanti ─────────
  const here = useCallback((): Loc => ({ view: S.current.view, openId: S.current.openId }), [S]);
  const record = useCallback(() => {
    const h = hist.current;
    h.back.push(here());
    if (h.back.length > 100) h.back.shift();
    h.fwd = [];
  }, [here]);
  const applyLoc = useCallback((l: Loc) => {
    if (viewKey(l.view) !== viewKey(S.current.view)) {
      pendingOpen.current = l.openId;
      if (l.view.kind === "search") setQuery(l.view.q);
      else setQuery("");
      setView(l.view);
    } else {
      setOpenId(l.openId);
      const i = l.openId ? S.current.rows.findIndex((r) => r.id === l.openId) : -1;
      if (i >= 0) setCursor(i);
    }
  }, [S]);
  const travel = useCallback((dir: -1 | 1) => {
    const h = hist.current;
    const l = dir < 0 ? h.back.pop() : h.fwd.pop();
    if (!l) return;
    (dir < 0 ? h.fwd : h.back).push(here());
    applyLoc(l);
    rerender();
  }, [here, applyLoc]);

  /** Il mouse sopra una riga della lista la seleziona (con il thread chiuso). */
  const hoverRow = useCallback((i: number) => {
    if (!S.current.openId && i < S.current.rows.length) setCursor(i);
  }, [S]);

  const openThread = useCallback((id: string) => {
    record();
    const i = S.current.rows.findIndex((r) => r.id === id);
    if (i >= 0) setCursor(i);
    setOpenId(id);
  }, [S, record]);

  const closeThread = useCallback(() => {
    record();
    setOpenId(null);
  }, [record]);

  const go = useCallback((v: View) => {
    if (viewKey(v) === viewKey(S.current.view) && !S.current.openId) return;
    record();
    if (v.kind !== "search") setQuery("");
    if (viewKey(v) === viewKey(S.current.view)) setOpenId(null);
    else setView(v);
  }, [S, record]);

  const focusSearch = useCallback(() => {
    if (S.current.openId) closeThread();
    setTimeout(() => searchRef.current?.focus(), 0);
  }, [S, closeThread]);

  const toggleContact = useCallback(() => {
    const v = !S.current.contactOn;
    setContactOn(v);
  }, [S]);

  const toggleRail = useCallback(() => {
    const v = !S.current.railOn;
    setRailOn(v);
    api.setSetting("ui.rail", v).catch(() => {});
  }, [S]);

  const setTheme = useCallback((t: "system" | "light" | "dark") => {
    applyTheme(t);
    rerender();
    api.setSetting("ui.theme", t).catch(() => {});
  }, []);
  const toggleTheme = useCallback(() => setTheme(isDark() ? "light" : "dark"), [setTheme]);
  const syncNow = useCallback(() => {
    api.syncNow().catch(() => {});
    toast("Sincronizzo…");
  }, [toast]);

  const closeOverlay = useCallback(() => {
    const was = S.current.overlay;
    setOverlay(null);
    if (was === "bulk-offer") api.setSetting("ui.bulk_offered", true).catch(() => {});
  }, [S]);

  const onBulkStarted = useCallback((op: number, n: number) => {
    closeOverlay();
    undoStack.current.push({ label: "Sgombero", run: () => api.bulkUndo(op) });
    toast(`${count(n)} thread segnati come fatti. Su Gmail continua in background.`, { undo: true });
    reload(true);
  }, [closeOverlay, toast, reload]);

  /** Sposta nell'altra scheda subito, poi chiede se ricordarlo per il mittente o il dominio. */
  const moveToOtherTab = useCallback(async () => {
    const s = S.current;
    if (!isSplit(s.view)) return;
    const ids = targets();
    if (!ids.length) return;
    const to: Split = s.view.kind === "important" ? "other" : "important";
    const first = s.rows.find((r) => r.id === ids[0]);
    const sender = first ? personOf(first.participants, me)?.email ?? null : null;
    removeRows(ids);
    try {
      await api.moveSplit(ids, to, "thread");
      setMoved({ ids, to, sender });
      setOverlay("remember");
    } catch (e) {
      toast(`Non riuscito: ${String(e)}`, { error: true });
      reload();
    }
  }, [S, targets, me, removeRows, toast, reload]);

  const remember = useCallback(async (how: Remember) => {
    const m = moved;
    setOverlay(null);
    setMoved(null);
    if (!m) return;
    const where = m.to === "other" ? "Altro" : "Importanti";
    if (how === "thread" || !m.sender) {
      toast(`Spostato in ${where}`);
      return;
    }
    try {
      await api.moveSplit(m.ids, m.to, how);
      const who = how === "sender" ? m.sender : `tutto @${m.sender.split("@")[1]}`;
      toast(`D'ora in poi ${who} va in ${where}`);
    } catch (e) {
      toast(`Non riuscito: ${String(e)}`, { error: true });
    }
  }, [moved, toast]);

  // ───────── posticipa ─────────
  const askSnooze = useCallback((ids?: string[]) => {
    const t = ids ?? targets();
    if (!t.length) return;
    setSnoozeIds(t);
    setOverlay("snooze");
  }, [targets]);

  const doSnooze = useCallback(async (when: number, label: string) => {
    const ids = snoozeIds;
    setOverlay(null);
    if (!ids.length) return;
    if (isInboxish(S.current.view)) removeRows(ids);
    else patchRows(ids, { in_inbox: false });
    try {
      const h = await api.snooze(ids, when);
      undoStack.current.push({ label: "Posticipo", run: () => api.unsnooze(h) });
      toast(`${ids.length > 1 ? `${count(ids.length)} thread tornano` : "Torna"} ${label}`, { undo: true });
    } catch (e) {
      toast(`Non riuscito: ${String(e)}`, { error: true });
      reload();
    }
  }, [snoozeIds, S, removeRows, patchRows, toast, reload]);

  // ───────── scrivi, rispondi, inoltra ─────────
  const startCompose = useCallback(async (mode: ComposeMode, threadId?: string) => {
    if (mode === "new") {
      setCompose({ draft: emptyDraft(), mode, inline: false, key: ++composeSeq });
      return;
    }
    const id = threadId ?? currentThread()?.id;
    if (!id) return;
    try {
      const d = await api.replyDraft(id, mode as ReplyMode);
      const inline = mode !== "forward";
      if (inline && S.current.openId !== id) {
        record();
        const i = S.current.rows.findIndex((r) => r.id === id);
        if (i >= 0) setCursor(i);
        setOpenId(id);
      }
      setCompose({ draft: d, mode, inline, key: ++composeSeq });
    } catch (e) {
      toast(`Non riesco a preparare la risposta: ${String(e)}`, { error: true });
    }
  }, [currentThread, record, toast, S]);

  const reopenDraft = useCallback((d: Draft) => {
    const inline = !!d.thread_id && d.thread_id === S.current.openId;
    setCompose({ draft: d, mode: d.thread_id ? "reply" : "new", inline, key: ++composeSeq });
  }, [S]);

  const cancelSend = useCallback(async (id: number) => {
    setSends((ss) => ss.filter((x) => x.id !== id));
    try {
      const d = await api.cancelSend(id);
      if (d) {
        reopenDraft(d);
        toast("Invio annullato: la bozza è di nuovo aperta");
      } else {
        toast("Troppo tardi: l'email è già partita");
      }
    } catch (e) {
      toast(`Annulla non riuscito: ${String(e)}`, { error: true });
    }
  }, [reopenDraft, toast]);

  const sendDraft = useCallback(async (d: Draft) => {
    setCompose(null);
    try {
      const q = await api.sendDraft(d, SEND_DELAY);
      undoStack.current.push({ label: "Invio", run: () => cancelSend(q.id) });
      setSends((ss) => [...ss.slice(-2), { id: q.id, at: q.send_at, state: "pending" }]);
    } catch (e) {
      toast(`Non inviata: ${String(e)}`, { error: true });
      reopenDraft(d);
    }
  }, [cancelSend, reopenDraft, toast]);

  // File trascinati sulla finestra senza un composer aperto: una nuova email che li ha già allegati.
  // Con il composer aperto li prende lui.
  useFileDrop(!compose, {
    over: () => {},
    drop: (paths) => {
      api.fileInfo(paths).then(
        (files) => files.length && setCompose({ draft: { ...emptyDraft(), attachments: files }, mode: "new", inline: false, key: ++composeSeq }),
        (e) => toast(String(e), { error: true }),
      );
    },
  });

  /** "Invia ora": senza aspettare la fine dei 30 secondi. */
  const sendNow = useCallback(async (id: number) => {
    setSends((ss) => ss.map((x) => (x.id === id ? { ...x, state: "now" } : x)));
    try {
      await api.sendNow(id);
    } catch (e) {
      toast(`Invia ora non riuscito: ${String(e)}`, { error: true });
    }
  }, [toast]);

  const closeCompose = useCallback((d: Draft) => {
    setCompose(null);
    if (d.body.trim() || d.attachments?.length) {
      toast("Bozza chiusa", { action: { label: "Riapri", run: () => reopenDraft(d) } });
    }
  }, [reopenDraft, toast]);

  // ───────── allegati ─────────
  const openAttachment = useCallback(async (messageId: string, partId: string, name: string) => {
    try {
      await api.openAttachment(messageId, partId);
    } catch (e) {
      toast(`Non riesco ad aprire «${name}»: ${String(e)}`, { error: true });
    }
  }, [toast]);

  const saveAttachment = useCallback(async (messageId: string, partId: string, name: string) => {
    try {
      const path = await api.saveAttachment(messageId, partId);
      const file = path.split("/").pop() ?? name;
      toast(`«${file}» salvato in Download`, { action: { label: "Mostra nel Finder", run: () => { api.revealDownload(path).catch(() => {}); } } });
    } catch (e) {
      toast(`Non riesco a salvare «${name}»: ${String(e)}`, { error: true });
    }
  }, [toast]);

  // ───────── sezioni ─────────
  const viewSection = useCallback((s: Section) => go(s.show_as_tab ? { kind: "section", id: s.label_id } : { kind: "label", id: s.label_id }), [go]);

  const newSection = useCallback((tab: boolean) => {
    setSectionEdit({ section: null, tab });
    setOverlay("section");
  }, []);

  const editSection = useCallback((s: Section) => {
    setMenu(null);
    setSectionEdit({ section: s, tab: s.show_as_tab });
    setOverlay("section");
  }, []);

  const saveSection = useCallback(async (v: { name: string; instructions: string | null; showAsTab: boolean }) => {
    const ed = sectionEdit;
    if (!ed) return;
    if (!ed.section) {
      const s = await api.createSection(v.name, v.showAsTab, v.instructions);
      setOverlay(null);
      setSectionEdit(null);
      await loadSections();
      viewSection(s);
      toast(v.instructions && aiOn ? `Sezione «${v.name}» creata: l'AI ci porta le email che corrispondono` : `Sezione «${v.name}» creata`);
    } else {
      const id = ed.section.label_id;
      if (v.name !== ed.section.name) await api.renameSection(id, v.name);
      await api.configureSection(id, v.showAsTab, v.instructions);
      setOverlay(null);
      setSectionEdit(null);
      await loadSections();
      const cur = S.current.view;
      if ((cur.kind === "section" || cur.kind === "label") && cur.id === id) viewSection({ ...ed.section, show_as_tab: v.showAsTab });
      toast("Sezione aggiornata");
    }
  }, [sectionEdit, loadSections, viewSection, toast, aiOn, S]);

  const toggleSectionTab = useCallback(async (s: Section) => {
    setMenu(null);
    setSections((cur) => cur.map((x) => (x.label_id === s.label_id ? { ...x, show_as_tab: !s.show_as_tab } : x)));
    try {
      await api.configureSection(s.label_id, !s.show_as_tab, s.instructions);
      const cur = S.current.view;
      if ((cur.kind === "section" || cur.kind === "label") && cur.id === s.label_id) viewSection({ ...s, show_as_tab: !s.show_as_tab });
    } catch (e) {
      toast(`Non riuscito: ${String(e)}`, { error: true });
      loadSections();
    }
  }, [S, viewSection, toast, loadSections]);

  const archiveSection = useCallback(async (s: Section) => {
    setMenu(null);
    const archived = !s.archived;
    setSections((cur) => cur.map((x) => (x.label_id === s.label_id ? { ...x, archived } : x)));
    const cur = S.current.view;
    if (archived && (cur.kind === "section" || cur.kind === "label") && cur.id === s.label_id) go({ kind: "important" });
    try {
      await api.archiveSection(s.label_id, archived);
      toast(archived ? `«${s.name}» archiviata: la trovi in fondo alla barra` : `«${s.name}» ripristinata`);
    } catch (e) {
      toast(`Non riuscito: ${String(e)}`, { error: true });
      loadSections();
    }
  }, [S, go, toast, loadSections]);

  const dropSection = useCallback(() => {
    const d = drag;
    setDrag(null);
    if (!d || d.at === null) return;
    const active = S.current.sections.filter((s) => !s.archived);
    const ids = active.map((s) => s.label_id);
    const from = ids.indexOf(d.id);
    if (from < 0) return;
    let to = d.at;
    ids.splice(from, 1);
    if (to > from) to--;
    ids.splice(to, 0, d.id);
    const order = [...ids, ...S.current.sections.filter((s) => s.archived).map((s) => s.label_id)];
    setSections((cur) => order.map((id, i) => ({ ...cur.find((s) => s.label_id === id)!, position: i })));
    api.reorderSections(order).catch((e) => {
      toast(`Ordine non salvato: ${String(e)}`, { error: true });
      loadSections();
    });
  }, [drag, S, toast, loadSections]);

  /** Dalle impostazioni: l'AI si è appena accesa (prima chiave) o è cambiata. */
  const aiChanged = useCallback((next: AiStatus) => {
    if (next.has_key && !S.current.ai?.has_key) toast("AI attiva: ⌘J nel composer scrive o sistema la bozza");
    setAi(next);
  }, [toast, S]);

  // ───────── registro dei comandi ─────────
  const viewKind = view.kind;
  const viewLabel = view.kind === "label" ? view.id : null;
  const tabViews = useMemo<View[]>(() => [
    { kind: "important" }, { kind: "other" },
    ...sections.filter((s) => s.show_as_tab && !s.archived).map((s) => ({ kind: "section" as const, id: s.label_id })),
  ], [sections]);
  const tabViewsRef = useLatest(tabViews);
  const cycleTab = useCallback((dir: 1 | -1) => {
    const tv = tabViewsRef.current;
    const i = tv.findIndex((v) => viewKey(v) === viewKey(S.current.view));
    go(tv[(i + dir + tv.length) % tv.length]);
  }, [go, S, tabViewsRef]);
  const commands = useMemo<Command[]>(() => {
    const cur = () => currentThread();
    const inInbox = () => {
      const s = S.current;
      if (!s.openId && s.picked.size) return s.rows.some((r) => s.picked.has(r.id) && r.in_inbox);
      return !!cur()?.in_inbox;
    };
    const has = () => targets().length > 0;
    return [
      // Thread
      { id: "done", title: "Fatto", group: "Thread", keys: ["e"], keywords: ["archivia", "done"], available: inInbox,
        run: () => act({ type: "archive" }, "Fatto", { removes: isInboxish(S.current.view) }) },
      { id: "snooze", title: "Posticipa (promemoria)", group: "Thread", keys: ["h"], keywords: ["ricorda", "reminder", "snooze", "più tardi"],
        available: inInbox, run: () => askSnooze() },
      { id: "compose", title: "Scrivi un'email", group: "Scrivi", keys: ["c"], keywords: ["nuova", "componi", "compose"], run: () => startCompose("new") },
      { id: "reply", title: "Rispondi", group: "Scrivi", keys: ["r"], keywords: ["reply"], available: has, run: () => startCompose("reply") },
      { id: "reply-all", title: "Rispondi a tutti", group: "Scrivi", keys: ["a"], keywords: ["reply all"], available: has, run: () => startCompose("reply_all") },
      { id: "forward", title: "Inoltra", group: "Scrivi", keys: ["f"], keywords: ["forward", "gira"], available: has, run: () => startCompose("forward") },
      { id: "to-inbox", title: "Rimetti in inbox", group: "Thread", keys: ["shift+e"], keywords: ["sposta in arrivo"],
        available: () => has() && !inInbox(),
        run: () => act({ type: "move_to_inbox" }, "Rimesso in inbox", { removes: viewKind === "done", patch: { in_inbox: true } }) },
      { id: "trash", title: "Cestina", group: "Thread", keys: ["#"], keywords: ["elimina", "cancella"],
        available: () => has() && viewLabel !== "TRASH",
        run: () => act({ type: "trash" }, "Spostato nel cestino", { removes: true }) },
      { id: "untrash", title: "Ripristina dal cestino", group: "Thread", keys: [], available: () => viewLabel === "TRASH" && has(),
        run: () => act({ type: "untrash" }, "Ripristinato", { removes: true }) },
      { id: "star", title: "Stella", group: "Thread", keys: ["s"], keywords: ["speciale", "preferito"], available: has,
        run: () => (cur()?.starred
          ? act({ type: "unstar" }, "Stella tolta", { patch: { starred: false } })
          : act({ type: "star" }, "Stella messa", { patch: { starred: true } })) },
      { id: "read", title: "Letto / non letto", group: "Thread", keys: ["u"], keywords: ["segna da leggere"], available: has,
        run: () => (cur()?.unread
          ? act({ type: "mark_read" }, "Segnato come letto", { patch: { unread: false } })
          : act({ type: "mark_unread" }, "Segnato come da leggere", { patch: { unread: true } })) },
      { id: "move-split", title: viewKind === "other" ? "Sposta in Importanti" : "Sposta in Altro", group: "Thread", keys: ["m"],
        keywords: ["importanti", "altro", "scheda", "correggi"], available: () => isSplit(S.current.view) && has(), run: moveToOtherTab },
      { id: "undo", title: "Annulla l'ultima azione", group: "Thread", keys: ["z", "mod+z"], run: undoLast },
      { id: "bulk", title: "Segna come fatti i thread più vecchi di…", group: "Thread", keys: [],
        keywords: ["sgombero", "inbox zero", "archivia vecchi"], run: () => setOverlay("bulk") },

      // Navigazione
      { id: "next", title: "Thread successivo", group: "Navigazione", keys: ["j", "arrowdown"], run: () => move(1) },
      { id: "prev", title: "Thread precedente", group: "Navigazione", keys: ["k", "arrowup"], run: () => move(-1) },
      { id: "open", title: "Apri il thread", group: "Navigazione", keys: ["enter"], contexts: ["list"],
        available: () => !!S.current.rows[S.current.cursor], run: () => openThread(S.current.rows[S.current.cursor].id) },
      { id: "back", title: "Torna alla lista", group: "Navigazione", keys: ["escape"], contexts: ["thread"], run: closeThread },
      { id: "hist-back", title: "Indietro", group: "Navigazione", keys: ["mod+["], available: () => hist.current.back.length > 0, run: () => travel(-1) },
      { id: "hist-fwd", title: "Avanti", group: "Navigazione", keys: ["mod+]"], available: () => hist.current.fwd.length > 0, run: () => travel(1) },
      { id: "search", title: "Cerca nella posta", group: "Navigazione", keys: ["/"], keywords: ["trova", "ricerca"], run: focusSearch },
      { id: "unpick", title: "Togli la selezione", group: "Navigazione", keys: ["escape"], contexts: ["list"],
        available: () => S.current.picked.size > 0, run: () => setPicked(new Set()) },
      { id: "pick", title: "Seleziona", group: "Navigazione", keys: ["x"], contexts: ["list"],
        available: () => !!S.current.rows[S.current.cursor],
        run: () => {
          const id = S.current.rows[S.current.cursor].id;
          setPicked((p) => {
            const n = new Set(p);
            if (n.has(id)) n.delete(id);
            else n.add(id);
            return n;
          });
        } },
      { id: "pick-down", title: "Estendi la selezione in giù", group: "Navigazione", keys: ["shift+j", "shift+arrowdown"], contexts: ["list"], run: () => move(1, true) },
      { id: "pick-up", title: "Estendi la selezione in su", group: "Navigazione", keys: ["shift+k", "shift+arrowup"], contexts: ["list"], run: () => move(-1, true) },
      ...[1, 2, 3, 4, 5].map((n) => ({
        id: `recent-${n}`, title: n === 1 ? "Apri una conversazione recente (da 1 a 5)" : `Apri la conversazione recente ${n}`,
        group: "Navigazione", keys: [String(n)], hidden: true, noHelp: n > 1,
        available: () => S.current.contactOn && recentThreads.current.length >= n,
        run: () => openThread(recentThreads.current[n - 1]),
      })),

      // Vai a
      { id: "tab-next", title: "Scheda successiva", group: "Vai a", keys: ["tab"], contexts: ["list"],
        available: () => isTab(S.current.view), run: () => cycleTab(1) },
      { id: "tab-prev", title: "Scheda precedente", group: "Vai a", keys: ["shift+tab"], contexts: ["list"], noHelp: true,
        available: () => isTab(S.current.view), run: () => cycleTab(-1) },
      { id: "go-inbox", title: "Importanti", group: "Vai a", keys: ["g i"], keywords: ["posta in arrivo", "inbox"], run: () => go({ kind: "important" }) },
      { id: "go-other", title: "Altro", group: "Vai a", keys: ["g o"], run: () => go({ kind: "other" }) },
      { id: "go-done", title: "Fatti", group: "Vai a", keys: ["g d"], keywords: ["archiviati"], run: () => go({ kind: "done" }) },
      { id: "go-sent", title: "Inviati", group: "Vai a", keys: ["g s"], run: () => go({ kind: "label", id: "SENT" }) },
      { id: "go-drafts", title: "Bozze", group: "Vai a", keys: ["g b"], run: () => go({ kind: "label", id: "DRAFT" }) },
      { id: "go-all", title: "Tutta la posta", group: "Vai a", keys: ["g a"], run: () => go({ kind: "all" }) },
      { id: "go-label", title: "Etichetta…", group: "Vai a", keys: ["g l"], keywords: ["label", "sezione"], run: () => setOverlay("labels") },
      { id: "go-snoozed", title: "Posticipati", group: "Vai a", keys: ["g h"], keywords: ["promemoria", "snooze"],
        available: () => !!S.current.labels.find((l) => isSnoozeLabel(l.name)),
        run: () => { const l = S.current.labels.find((x) => isSnoozeLabel(x.name)); if (l) go({ kind: "label", id: l.id }); } },
      { id: "new-section", title: "Nuova sezione", group: "Sezioni", keys: [], keywords: ["etichetta", "crea", "scheda", "filtro"], run: () => newSection(false) },
      { id: "new-tab", title: "Nuova sezione come scheda in alto", group: "Sezioni", keys: [], keywords: ["scheda", "tab", "split"], run: () => newSection(true) },

      // App
      { id: "palette", title: "Comandi", group: "App", keys: ["mod+k"], hidden: true, run: () => setOverlay("palette") },
      { id: "help", title: "Scorciatoie", group: "App", keys: ["?"], keywords: ["aiuto", "tasti"], run: () => setOverlay("help") },
      { id: "contact", title: "Barra del contatto", group: "App", keys: ["i"], keywords: ["persona", "info"], run: toggleContact },
      { id: "restart", title: "Riavvia diMail", group: "App", keys: [], keywords: ["riapri", "restart", "aggiornamento"], run: restartForUpdate },
      { id: "rail", title: "Barra a sinistra", group: "App", keys: ["mod+\\"], run: toggleRail },
      { id: "sync", title: "Sincronizza ora", group: "App", keys: [], keywords: ["aggiorna"], run: syncNow },
      { id: "settings", title: "Impostazioni", group: "App", keys: ["mod+,"], keywords: ["preferenze", "domini", "client", "google"], run: () => setOverlay("settings") },
      { id: "ai-key", title: "AI: provider, chiave e modello", group: "App", keys: [], keywords: ["ai", "mistral", "openai", "modello", "scrivi", "bozza"], run: () => setOverlay("ai-key") },
      { id: "theme-toggle", title: "Tema chiaro o scuro", group: "App", keys: [], keywords: ["notte", "giorno"], run: toggleTheme },
      { id: "rules", title: "Rivedi le correzioni di Importanti e Altro", group: "App", keys: [], keywords: ["regole", "mittenti"],
        run: () => { api.splitRules().then((r) => { setRules(r); setOverlay("rules"); }).catch(() => {}); } },
      { id: "theme-system", title: "Tema: come il Mac", group: "App", keys: [], run: () => setTheme("system") },
      { id: "theme-light", title: "Tema: chiaro", group: "App", keys: [], run: () => setTheme("light") },
      { id: "theme-dark", title: "Tema: scuro", group: "App", keys: [], run: () => setTheme("dark") },
    ];
  }, [S, act, askSnooze, closeThread, currentThread, cycleTab, focusSearch, go, move, moveToOtherTab, newSection, openThread, restartForUpdate, setTheme, startCompose, syncNow, targets, toggleContact, toggleRail, toggleTheme, travel, undoLast, viewKind, viewLabel]);

  const commandsRef = useLatest(commands);
  const ctx: Context = openId ? "thread" : "list";

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (S.current.overlay) return; // le sovrapposizioni gestiscono i propri tasti
      if (e.defaultPrevented) return;
      if (router.current.handle(e, commandsRef.current, S.current.openId ? "thread" : "list")) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [S, commandsRef]);

  // ───────── cosa mostrare ─────────
  const labelName = useCallback(
    (id: string) => SYSTEM_NAMES[id] ?? labels.find((l) => l.id === id)?.name ?? null,
    [labels],
  );
  const nameOf = (v: View): string =>
    v.kind === "inbox" ? "Posta in arrivo" : v.kind === "important" ? "Importanti"
    : v.kind === "other" ? "Altro" : v.kind === "done" ? "Fatti" : v.kind === "all"
    ? "Tutta la posta" : v.kind === "search" ? `Cerca: ${v.q}`
    : isSnoozeLabel(labels.find((l) => l.id === v.id)?.name) ? "Posticipati" : (labelName(v.id) ?? v.id);
  const viewName = nameOf(view);
  const inboxUnread = labels.find((l) => l.id === "INBOX")?.unread_local ?? 0;
  useEffect(() => {
    document.title = inboxUnread > 0 ? `diMail · ${count(inboxUnread)} da leggere` : "diMail";
  }, [inboxUnread]);

  // Il numero sull'icona nel Dock: le conversazioni in Importanti, come nella barra laterale.
  const importantCount = tabCounts.important;
  useEffect(() => {
    if (importantCount === undefined) return;
    try {
      getCurrentWindow().setBadgeCount(importantCount > 0 ? importantCount : undefined).catch(() => {});
    } catch {
      // nel browser (sviluppo) non c'è la finestra di Tauri
    }
  }, [importantCount]);

  const current = openId ? rows.find((r) => r.id === openId) : rows[cursor];
  const openDetail = openId ? details.current.get(openId) ?? null : null;
  const anyDetail = current ? details.current.get(current.id) : undefined;
  const lastMsg = anyDetail?.messages[anyDetail.messages.length - 1];
  // Un thread aperto che non è nella lista e non è ancora arrivato: la barra del contatto tiene la
  // persona di prima invece di passare per "Nessun contatto" (11 novembre).
  const lastPerson = useRef<Addr | null>(null);
  const person = openId && !current && !openDetail
    ? lastPerson.current
    : personOf(current?.participants ?? openDetail?.thread.participants ?? [], me, lastMsg?.to[0] ?? null);
  lastPerson.current = person;
  const onCard = useCallback((c: ContactCard | null) => {
    recentThreads.current = c?.recent.map((t) => t.id) ?? [];
  }, []);

  const paletteItems = useMemo<PaletteItem[]>(() => {
    const labelItems: PaletteItem[] = labels
      .filter((l) => (l.kind === "user" && l.label_list_visibility !== "labelHide") || ["STARRED", "IMPORTANT", "SPAM", "TRASH"].includes(l.id))
      .map((l) => ({ id: `label:${l.id}`, title: SYSTEM_NAMES[l.id] ?? l.name, group: "Etichetta", run: () => go({ kind: "label", id: l.id }) }));
    if (overlay === "labels") return labelItems;
    const cmds = available(commands, ctx)
      .filter((c) => !c.hidden)
      .map((c) => ({ id: c.id, title: c.title, group: c.group, keys: c.keys, keywords: c.keywords, run: c.run }));
    return [...cmds, ...labelItems];
  }, [commands, ctx, labels, overlay, go]);

  const syncing = !!status && (status.phase === "initial" || status.phase === "syncing");
  const statusText = (() => {
    if (bulk && (bulk.state === "listing" || bulk.state === "applying")) {
      return bulk.total ? `Sgombero su Gmail: ${count(bulk.done)} di ${count(bulk.total)}` : "Sgombero su Gmail: elenco i messaggi…";
    }
    if (!status) return "";
    switch (status.phase) {
      case "initial": return `Prima sincronizzazione: ${count(status.threads_local)} thread sul Mac, ${count(status.queued + status.queued_background)} da scaricare`;
      case "syncing": return "Sincronizzo…";
      case "offline": return "Offline: le azioni partono al ritorno della rete";
      case "auth_required": return "Accesso a Google scaduto";
      case "error": return "Errore di sincronizzazione";
      default: return status.last_sync_at ? `Aggiornato alle ${clock(status.last_sync_at)}` : "";
    }
  })();

  const inboxZero = (view.kind === "inbox" || view.kind === "important") && loaded && rows.length === 0 && total === 0;
  const snoozeLabelRow = labels.find((l) => isSnoozeLabel(l.name));

  const attention = !!status && ["offline", "error", "auth_required"].includes(status.phase);
  const bulkRunning = !!bulk && (bulk.state === "listing" || bulk.state === "applying");
  const activeSections = sections.filter((s) => !s.archived);
  const archivedSections = sections.filter((s) => s.archived);
  const sectionOf = (id: string) => sections.find((s) => s.label_id === id);
  const viewIcon = (v: View, size = 17) => {
    switch (v.kind) {
      case "important": case "inbox": return <I.Inbox size={size} />;
      case "other": return <I.Stack size={size} />;
      case "done": return <I.DoneAll size={size} />;
      case "all": return <I.Stack size={size} />;
      case "search": return <I.Search size={size} />;
      case "section": return sectionOf(v.id)?.instructions ? <I.Sparkle size={size} /> : <I.Tag size={size} />;
      case "label": return v.id === "SENT" ? <I.Send size={size} /> : v.id === "DRAFT" ? <I.Draft size={size} />
        : v.id === snoozeLabelRow?.id ? <I.Alarm size={size} /> : <I.Tag size={size} />;
    }
  };
  // Le schede in alto: Importanti, Altro, le sezioni come scheda, più la vista corrente se è un'altra.
  const topTabs: { key: string; title: string; icon: React.ReactNode; n?: number; on: boolean; onClick: () => void }[] = [
    ...tabViews.map((v) => ({
      key: viewKey(v), title: nameOf(v), icon: viewIcon(v), n: tabCounts[viewKey(v)],
      on: viewKey(view) === viewKey(v), onClick: () => go(v),
    })),
    ...(tabViews.some((v) => viewKey(v) === vk) ? [] : [{ key: vk, title: viewName, icon: viewIcon(view), n: loaded ? total : undefined, on: true, onClick: () => go(view) }]),
  ];
  const menuSection = menu ? sectionOf(menu.id) : undefined;
  const replyNode = compose && compose.inline && compose.draft.thread_id === openId
    ? <Composer key={compose.key} initial={compose.draft} mode={compose.mode} inline me={me} onSend={sendDraft} onClose={closeCompose} onAiWrite={aiWrite} />
    : null;
  const sectionRow = (s: Section, i: number, archivedRow = false) => (
    <div key={s.label_id}
      className={`sec-row${drag?.id === s.label_id ? " dragging" : ""}${!archivedRow && drag?.at === i ? " drop-before" : ""}${!archivedRow && drag?.at === i + 1 && i === activeSections.length - 1 ? " drop-after" : ""}`}
      draggable={!archivedRow}
      onDragStart={(e) => { e.dataTransfer.effectAllowed = "move"; e.dataTransfer.setData("text/plain", s.label_id); setDrag({ id: s.label_id, at: null }); }}
      onDragOver={(e) => {
        if (archivedRow || !drag) return;
        e.preventDefault();
        const r = e.currentTarget.getBoundingClientRect();
        const at = e.clientY < r.top + r.height / 2 ? i : i + 1;
        if (drag.at !== at) setDrag({ ...drag, at });
      }}
      onDrop={(e) => { e.preventDefault(); dropSection(); }}
      onDragEnd={() => setDrag(null)}
      onDoubleClick={() => editSection(s)}
      onContextMenu={(e) => { e.preventDefault(); setMenu({ id: s.label_id, top: e.clientY, left: e.clientX }); }}>
      <NavItem label={s.name} n={s.unread || undefined}
        active={(view.kind === "label" || view.kind === "section") && view.id === s.label_id}
        icon={s.instructions ? <I.Sparkle size={16} style={{ color: s.color_bg ?? "var(--ai)" }} />
          : <i className="label-swatch" style={{ background: s.color_bg ?? "var(--line-2)" }} />}
        onClick={() => viewSection(s)} />
      <button className="sec-more" title="Opzioni della sezione"
        onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setMenu({ id: s.label_id, top: r.bottom + 4, left: r.left - 150 }); }}>
        <I.Dots size={16} />
      </button>
    </div>
  );
  const dark = isDark();

  return (
    <div className="app">
      {railOn && (
        <aside className="sidebar" aria-label="Viste">
          <div className="sb-drag" data-tauri-drag-region />
          <nav className="sb-nav">
            <NavItem icon={<I.Inbox size={18} />} label="Posta in arrivo" n={tabCounts.important} active={isSplit(view)} onClick={() => go({ kind: "important" })} />
            {snoozeLabelRow && (
              <NavItem icon={<I.Alarm size={18} />} label="Posticipati" active={viewLabel === snoozeLabelRow.id}
                onClick={() => go({ kind: "label", id: snoozeLabelRow.id })} />
            )}
            <NavItem icon={<I.DoneAll size={18} />} label="Fatti" active={view.kind === "done"} onClick={() => go({ kind: "done" })} />
            <NavItem icon={<I.Send size={18} />} label="Inviati" active={viewLabel === "SENT"} onClick={() => go({ kind: "label", id: "SENT" })} />
            <NavItem icon={<I.Draft size={18} />} label="Bozze" active={viewLabel === "DRAFT"} onClick={() => go({ kind: "label", id: "DRAFT" })} />
            <NavItem icon={<I.Stack size={18} />} label="Tutta la posta" active={view.kind === "all"} onClick={() => go({ kind: "all" })} />
          </nav>
          <div className="sb-section">
            <span>Sezioni</span>
            <button className="mini" onClick={() => newSection(false)} title="Nuova sezione">
              <I.Plus size={15} />
            </button>
          </div>
          <div className="sb-labels" onDragOver={(e) => drag && e.preventDefault()} onDrop={(e) => { e.preventDefault(); dropSection(); }}>
            {activeSections.map((s, i) => sectionRow(s, i))}
            {activeSections.length === 0 && (
              <button className="sb-empty" onClick={() => newSection(false)}>
                Crea una sezione e dì all'AI cosa ci va
              </button>
            )}
            {archivedSections.length > 0 && (
              <>
                <button className="sb-archived" onClick={() => setArchivedOpen((o) => !o)}>
                  <I.Archive size={15} /> Archiviate <span className="n">{archivedSections.length}</span>
                  <I.ChevronRight size={14} style={{ transform: archivedOpen ? "rotate(90deg)" : undefined, transition: "transform .18s" }} />
                </button>
                {archivedOpen && <div className="archived-list">{archivedSections.map((s, i) => sectionRow(s, i, true))}</div>}
              </>
            )}
          </div>
          <div className="account-pill" title={me}>
            <span className="avatar" style={{ background: avatarColor(me) }}>{initials(account.name, me)}</span>
            {(account.name ?? me).split(" ")[0]}
          </div>
        </aside>
      )}

      <section className={railOn ? "main" : "main rail-off"}>
        <header className="topbar" data-tauri-drag-region>
          <button className="icon-btn" onClick={toggleRail} title="Barra a sinistra (⌘\)">
            <I.PanelLeft size={19} />
          </button>
          <button className="icon-btn" onClick={() => travel(-1)} disabled={hist.current.back.length === 0} title="Indietro (⌘[)">
            <I.ChevronLeft size={19} />
          </button>
          <button className="icon-btn" onClick={() => travel(1)} disabled={hist.current.fwd.length === 0} title="Avanti (⌘])">
            <I.ChevronRight size={19} />
          </button>
          <div className="tabs" role="tablist">
            {topTabs.map((t) => (
              <button key={t.key} role="tab" aria-selected={t.on} className={t.on ? "tab on" : "tab"} onClick={t.onClick}
                title={isTab(view) ? `${t.title} · Tab per cambiare scheda` : t.title}
                onContextMenu={(e) => {
                  const id = t.key.startsWith("section:") ? t.key.slice(8) : null;
                  if (!id) return;
                  e.preventDefault();
                  setMenu({ id, top: e.clientY, left: e.clientX });
                }}>
                {t.icon}
                <span className="t">{t.title}</span>
                {t.n !== undefined && <span className="n">{count(t.n)}</span>}
              </button>
            ))}
          </div>
          <button className="icon-btn" onClick={() => newSection(true)} title="Nuova sezione come scheda (dì all'AI cosa ci va)">
            <I.Plus size={19} />
          </button>
          <span className="grow" data-tauri-drag-region />
          <button className={`icon-btn sync-btn${syncing || bulkRunning ? " spinning" : ""}`} onClick={syncNow}
            title={statusText ? `${statusText}. Clic per sincronizzare ora` : "Sincronizza ora"}>
            <I.Refresh size={18} />
            {attention && <i className="badge-dot" />}
          </button>
          <button className="icon-btn" onClick={toggleTheme} title={dark ? "Tema chiaro" : "Tema scuro"}>
            {dark ? <I.Sun size={18} /> : <I.Moon size={18} />}
          </button>
        </header>

        {authLost && (
          <div className="banner">
            L'accesso a Google è scaduto: la posta locale resta leggibile, le azioni partono dopo il login.
            <Login hint={me} compact onDone={() => { setAuthLost(false); onRelogin(); }} />
          </div>
        )}

        {openId ? (
          <ThreadView key={openId} detail={openDetail} summary={current} me={me} myName={account.name} labelName={labelName}
            contactOn={contactOn} reply={replyNode} onReply={(m) => startCompose(m, openId)}
            onOpenAttachment={openAttachment} onSaveAttachment={saveAttachment}
            onPalette={() => setOverlay("palette")} onSearch={focusSearch} onToggleContact={toggleContact} />
        ) : (
          <>
            <div className="toolbar">
              <label className="search">
                <I.Search size={18} />
                <input ref={searchRef} value={query} placeholder="Cerca nella posta" spellCheck={false}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.preventDefault();
                      setQuery("");
                      e.currentTarget.blur();
                    } else if (e.key === "Enter" || e.key === "ArrowDown") {
                      e.preventDefault();
                      e.currentTarget.blur();
                    }
                  }} />
              </label>
              {picked.size > 0 && <span className="pill">{count(picked.size)} selezionati</span>}
              {ai && !ai.has_key && (
                <button className="ai-pill" onClick={() => setOverlay("ai-key")} title="Risposte scritte con ⌘J e sezioni smistate dall'AI">
                  <I.Sparkle size={15} /> Attiva l'AI
                </button>
              )}
              <button className="icon-btn" onClick={() => setOverlay("bulk")} title="Segna come fatti i thread vecchi">
                <I.DoneAll size={18} />
              </button>
              <button className={`icon-btn${contactOn ? " on" : ""}`} onClick={toggleContact} title="Barra del contatto (I)">
                <I.PanelRight size={18} />
              </button>
              <button className="icon-btn" onClick={() => setOverlay("help")} title="Scorciatoie (?)">
                <I.Help size={18} />
              </button>
            </div>
            {inboxZero ? (
              <div className="empty">
                <div className="zero">
                  <h3>Fatto tutto</h3>
                  <p>{doneToday > 0 ? `Oggi hai chiuso ${count(doneToday)} thread.` : "Niente da fare in Importanti."}</p>
                </div>
              </div>
            ) : loaded && rows.length === 0 ? (
              <div className="empty"><p className="empty-plain">{view.kind === "search" ? `Nessun thread per «${view.q}».` : "Niente qui."}</p></div>
            ) : (
              <ThreadList key={vk} listKey={vk} rows={rows} total={total} cursor={cursor} picked={picked} me={me}
                onOpen={(i) => rows[i] && openThread(rows[i].id)}
                onHover={hoverRow}
                onDone={(i) => rows[i] && act({ type: "archive" }, "Fatto", { removes: isInboxish(view), ids: [rows[i].id] })}
                onSnooze={(i) => rows[i] && askSnooze([rows[i].id])}
                onTrash={(i) => rows[i] && act({ type: "trash" }, "Spostato nel cestino", { removes: true, ids: [rows[i].id] })}
                onNeedMore={loadMore} />
            )}
          </>
        )}

        {compose && !compose.inline && (
          <Composer key={compose.key} initial={compose.draft} mode={compose.mode} me={me} onSend={sendDraft} onClose={closeCompose} onAiWrite={aiWrite} />
        )}
        {!openId && !(compose && !compose.inline) && (
          <button className="fab" onClick={() => startCompose("new")} title="Scrivi un'email (C)">
            <I.Pencil size={22} />
            <span>Scrivi</span>
          </button>
        )}
      </section>

      {contactOn && <ContactBar person={person} onOpenThread={openThread} onCard={onCard} onClose={toggleContact} />}

      {menu && menuSection && (
        <>
          <div className="overlay clear" onClick={() => setMenu(null)} onContextMenu={(e) => { e.preventDefault(); setMenu(null); }} />
          <div className="popover menu" style={{ top: Math.min(menu.top, window.innerHeight - 180), left: Math.max(8, Math.min(menu.left, window.innerWidth - 240)) }}>
            <button onClick={() => editSection(menuSection)}><I.Pencil size={15} /> Rinomina e istruzioni AI…</button>
            {!menuSection.archived && (
              <button onClick={() => toggleSectionTab(menuSection)}>
                <I.Stack size={15} /> {menuSection.show_as_tab ? "Togli dalle schede in alto" : "Mostra come scheda in alto"}
              </button>
            )}
            <button onClick={() => archiveSection(menuSection)}>
              <I.Archive size={15} /> {menuSection.archived ? "Ripristina" : "Archivia"}
            </button>
          </div>
        </>
      )}

      {overlay === "snooze" && <SnoozePicker count={snoozeIds.length} onPick={doSnooze} onClose={closeOverlay} />}
      {overlay === "section" && sectionEdit && (
        <SectionDialog section={sectionEdit.section} defaultTab={sectionEdit.tab} aiOn={aiOn} onSave={saveSection}
          onSetupAi={() => setOverlay("ai-key")} onClose={() => { setOverlay(null); setSectionEdit(null); }} />
      )}
      {(overlay === "ai-key" || overlay === "settings") && (
        <SettingsDialog tab={"ai" satisfies SettingsTab} onAiChanged={aiChanged}
          onClientChanged={onRelogin} onClose={closeOverlay} />
      )}

      {(overlay === "palette" || overlay === "labels") && (
        <Palette items={paletteItems} onClose={closeOverlay} placeholder={overlay === "labels" ? "Vai all'etichetta…" : undefined} />
      )}
      {overlay === "help" && <Help commands={available(commands, ctx)} onClose={closeOverlay} />}
      {overlay === "remember" && moved && (
        <Palette
          onClose={() => setOverlay(null)}
          onCancel={() => remember("thread")}
          placeholder={`Spostato in ${moved.to === "other" ? "Altro" : "Importanti"}. Ricordarlo?`}
          items={[
            { id: "r-thread", title: "Solo questo thread", group: "Ricorda", run: () => remember("thread") },
            ...(moved.sender
              ? [
                  { id: "r-sender", title: `Sempre per ${moved.sender}`, group: "Ricorda", run: () => remember("sender") },
                  { id: "r-domain", title: `Sempre per tutto @${moved.sender.split("@")[1]}`, group: "Ricorda", run: () => remember("domain") },
                ]
              : []),
          ]}
        />
      )}
      {overlay === "rules" && (
        <Palette
          onClose={closeOverlay}
          placeholder={rules.length ? "Invio toglie la correzione selezionata" : "Nessuna correzione ricordata"}
          items={rules.map((r) => ({
            id: `${r.kind}:${r.pattern}`,
            title: `${r.kind === "domain" ? `tutto @${r.pattern}` : r.pattern} → ${r.split === "other" ? "Altro" : "Importanti"}`,
            group: "Togli",
            run: () => {
              api.removeSplitRule(r.kind, r.pattern).then(() => toast(`Correzione tolta: ${r.pattern}`)).catch((e) => toast(String(e), { error: true }));
            },
          }))}
        />
      )}
      {(overlay === "bulk" || overlay === "bulk-offer") && (
        <BulkDialog offer={overlay === "bulk-offer"} onClose={closeOverlay} onStarted={onBulkStarted} />
      )}

      <div className={compose && (!compose.inline || compose.draft.thread_id === openId) ? "toasts aside" : "toasts"} aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={t.error ? "toast error" : "toast"}>
            <span>{t.text}</span>
            {t.undo && (
              <button onClick={() => { dismissToast(t.id); undoLast(); }}>
                Annulla <kbd>Z</kbd>
              </button>
            )}
            {t.action && (
              <button onClick={() => { dismissToast(t.id); t.action!.run(); }}>{t.action.label}</button>
            )}
            <button className="x" onClick={() => dismissToast(t.id)} aria-label="Chiudi">×</button>
          </div>
        ))}
      </div>

      <div className="send-pops" aria-live="polite">
        {update.version && (
          <div className="send-pop update">
            <div className="sp-row">
              <I.Refresh size={15} />
              <span className="sp-text" title={`diMail ${update.version} è pronta`}>diMail {update.version} è pronta</span>
              <button className="sp-undo" onClick={restartForUpdate} title="Riavvia diMail con la nuova versione">Riavvia</button>
            </div>
          </div>
        )}
        {sends.map((x) => <SendPop key={x.id} s={x} onUndo={() => cancelSend(x.id)} onNow={() => sendNow(x.id)} />)}
      </div>
    </div>
  );
}

/** In basso a sinistra, per ogni email in coda: il conto alla rovescia, una barra che si svuota e
 *  Annulla (o Z); passandoci sopra compare "Invia ora". Poi "Inviata". */
function SendPop({ s, onUndo, onNow }: { s: Sending; onUndo: () => void; onNow: () => void }) {
  const [, tick] = useReducer((x: number) => x + 1, 0);
  const [bar] = useState(() => {
    const ms = Math.max(0, s.at - Date.now());
    return { ms, from: Math.min(1, ms / SEND_DELAY) };
  });
  const waiting = s.state === "pending";
  useEffect(() => {
    if (!waiting) return;
    const t = setInterval(tick, 250);
    return () => clearInterval(t);
  }, [waiting]);
  const left = Math.ceil((s.at - Date.now()) / 1000);
  const counting = waiting && left > 0;
  const text = counting ? <>Invio tra <b className="num">{left}</b> s</>
    : s.state === "sent" ? "Inviata"
    : s.state === "retrying" ? "Gmail non risponde: riprovo"
    : "Invio…";
  return (
    <div className={`send-pop ${s.state}${counting ? " counting" : ""}`}>
      <div className="sp-row">
        {s.state === "sent" ? <I.Check size={16} /> : <I.Send size={15} />}
        <span className="sp-text">{text}</span>
        {counting && (
          <button className="sp-undo" onClick={onUndo} title="Annulla (Z)">Annulla</button>
        )}
      </div>
      {counting && (
        <div className="sp-more">
          <div>
            <button className="sp-now" onClick={onNow}>
              <I.Send size={14} /> Invia ora
            </button>
          </div>
        </div>
      )}
      {counting && <i className="sp-bar" style={{ "--from": bar.from, animationDuration: `${bar.ms}ms` } as React.CSSProperties} />}
    </div>
  );
}

function NavItem({ icon, label, n, active, onClick }: { icon: React.ReactNode; label: string; n?: number; active?: boolean; onClick: () => void }) {
  return (
    <button className={active ? "nav-item active" : "nav-item"} onClick={onClick}>
      {icon}
      <span className="label">{label}</span>
      {n !== undefined && n > 0 && <span className="n">{count(n)}</span>}
    </button>
  );
}

function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  return t ? t === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyTheme(t: string) {
  if (t === "light" || t === "dark") document.documentElement.dataset.theme = t;
  else delete document.documentElement.dataset.theme;
}
