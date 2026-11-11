// Lista dei thread a righe compatte, come Superhuman (richiesta di Tommaso, 9 novembre): su ogni riga
// prima chi scrive, poi l'oggetto, poi l'inizio del testo, a destra allegati e ora. Sulla riga selezionata
// l'ora lascia il posto a Fatto, Promemoria e Cestina. C'è una sola riga evidenziata: muovendo il mouse
// (o scorrendo con il trackpad) la selezione va sulla riga sotto il puntatore, con J e K si sposta da
// tastiera; E, H e # agiscono sempre su quella (9 novembre, sera). Raggruppate per data:
// Oggi, Ieri, Ultimi 7 giorni, poi i mesi. Virtualizzata: si disegnano solo le righe visibili. Quando
// una riga esce (E, posticipa, cestina) sfuma e le altre salgono al suo posto.
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ThreadSummary } from "./api";
import { dayGroup, participantsLabel, startOfToday, whenLabel } from "./format";
import { Alarm, Check, Clip, Star, Trash } from "./icons";

type Props = {
  rows: ThreadSummary[];
  total: number;
  cursor: number;
  picked: Set<string>;
  me: string;
  /** Cambia con la vista: al cambio niente righe in uscita. */
  listKey: string;
  onOpen: (index: number) => void;
  /** Il mouse è sopra questa riga: diventa la selezionata. */
  onHover: (index: number) => void;
  onDone: (index: number) => void;
  onSnooze: (index: number) => void;
  onTrash: (index: number) => void;
  onNeedMore: () => void;
};

/** Altezza di una riga. */
export const ROW_H = 46;
const TOP = 4;
const OVERSCAN = 10;
const GHOST_MS = 260;
/** Altezza dell'intestazione di un gruppo (Oggi, Ieri…). */
const HEAD_H = 46;

type Layout = { tops: number[]; heads: { key: string; label: string; top: number }[]; height: number; recent: boolean[] };

/** Posizione di ogni riga e delle intestazioni dei gruppi. I posticipati tornati stanno in "Oggi". */
function layoutOf(rows: ThreadSummary[], today: number): Layout {
  const tops: number[] = new Array(rows.length);
  const recent: boolean[] = new Array(rows.length);
  const heads: Layout["heads"] = [];
  let y = TOP;
  let prev = "";
  rows.forEach((t, i) => {
    const g = dayGroup(t.reminded ? Math.max(t.last_date, today) : t.last_date, today);
    if (g.key !== prev) {
      // Il primo gruppo (di solito Oggi) non ha bisogno di spazio sopra.
      heads.push({ key: g.key, label: g.label, top: y });
      y += i === 0 ? HEAD_H - 10 : HEAD_H;
      prev = g.key;
    }
    tops[i] = y;
    recent[i] = g.key === "today" || g.key === "yesterday";
    y += ROW_H;
  });
  return { tops, heads, height: y, recent };
}

/** Primo indice con `tops[i] >= y` (ricerca binaria). */
function firstAtOrAfter(tops: number[], y: number): number {
  let lo = 0;
  let hi = tops.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (tops[mid] < y) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

type Ghost = { key: number; t: ThreadSummary; top: number; recent: boolean };
let ghostSeq = 0;

export function ThreadList({ rows, total, cursor, picked, me, listKey, onOpen, onHover, onDone, onSnooze, onTrash, onNeedMore }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const cursorNow = useRef(cursor);
  cursorNow.current = cursor;
  /** Dov'è il puntatore e quando è arrivato l'ultimo evento della rotella (o del trackpad). */
  const pointer = useRef({ x: -1, y: -1, inside: false, wheel: 0 });
  /** La selezione messa dal mouse: non serve farla scorrere in vista. */
  const fromMouse = useRef({ i: -1, at: 0 });

  const hoverAt = useCallback((el: Element | null) => {
    const row = el?.closest?.("[data-i]") as HTMLElement | null;
    const i = row ? Number(row.dataset.i) : -1;
    if (i < 0 || i === cursorNow.current) return;
    fromMouse.current = { i, at: Date.now() };
    onHover(i);
  }, [onHover]);
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(800);
  const [ghosts, setGhosts] = useState<Ghost[]>([]);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    setHeight(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  const today = startOfToday();
  const layout = useMemo(() => layoutOf(rows, today), [rows, today]);

  // La riga selezionata resta sempre visibile, con un margine che lascia vedere anche l'intestazione
  // del gruppo.
  useLayoutEffect(() => {
    const m = fromMouse.current;
    fromMouse.current = { i: -1, at: 0 };
    if (m.i === cursor && Date.now() - m.at < 1000) return;
    const el = box.current;
    if (!el || cursor < 0 || cursor >= layout.tops.length) return;
    const top = cursor === 0 ? 0 : layout.tops[cursor];
    const margin = HEAD_H + 8;
    if (top - margin < el.scrollTop) el.scrollTop = Math.max(0, top - margin);
    else if (top + ROW_H + margin > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_H + margin - el.clientHeight;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cursor]);

  const first = Math.max(0, firstAtOrAfter(layout.tops, scroll - ROW_H) - OVERSCAN);
  const last = Math.min(rows.length, firstAtOrAfter(layout.tops, scroll + height) + OVERSCAN);

  // Righe uscite dalla lista (e visibili): restano un attimo, sfumando, al loro posto.
  const prev = useRef<{ key: string; rows: ThreadSummary[]; layout: Layout; first: number; last: number } | null>(null);
  useLayoutEffect(() => {
    const p = prev.current;
    prev.current = { key: listKey, rows, layout, first, last };
    if (!p || p.key !== listKey || p.rows === rows) return;
    const now = new Set(rows.map((r) => r.id));
    const gone: Ghost[] = [];
    for (let i = p.first; i < Math.min(p.last, p.rows.length); i++) {
      const t = p.rows[i];
      if (!now.has(t.id)) gone.push({ key: ++ghostSeq, t, top: p.layout.tops[i], recent: p.layout.recent[i] });
    }
    if (gone.length === 0 || gone.length > 12) return;
    setGhosts((g) => [...g, ...gone]);
    const keys = new Set(gone.map((g) => g.key));
    setTimeout(() => setGhosts((g) => g.filter((x) => !keys.has(x.key))), GHOST_MS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, listKey]);

  useEffect(() => {
    if (rows.length < total && (last >= rows.length - OVERSCAN || cursor >= rows.length - 20)) onNeedMore();
  }, [last, cursor, rows.length, total, onNeedMore]);

  const onScroll = useCallback(() => {
    setScroll(box.current?.scrollTop ?? 0);
    // Scorrendo con il trackpad le righe passano sotto il puntatore fermo: la selezione le segue. Lo
    // scorrimento fatto da J e K (riga selezionata da tenere in vista) non arriva dalla rotella e non conta.
    const p = pointer.current;
    if (p.inside && Date.now() - p.wheel < 300) requestAnimationFrame(() => hoverAt(document.elementFromPoint(p.x, p.y)));
  }, [hoverAt]);

  const onMouseMove = useCallback((e: React.MouseEvent) => {
    const p = pointer.current;
    p.inside = true;
    // Dopo uno scorrimento WebKit manda un movimento finto, con il puntatore fermo: non conta.
    if (e.clientX === p.x && e.clientY === p.y) return;
    p.x = e.clientX;
    p.y = e.clientY;
    hoverAt(e.target as Element);
  }, [hoverAt]);

  const items = [];
  for (let i = first; i < last; i++) {
    const t = rows[i];
    items.push(
      <Row key={t.id} t={t} index={i} top={layout.tops[i]} me={me} recent={layout.recent[i]}
        selected={i === cursor} picked={picked.has(t.id)} onOpen={onOpen} onDone={onDone} onSnooze={onSnooze} onTrash={onTrash} />,
    );
  }
  const margin = ROW_H * OVERSCAN;

  return (
    <div className="cards rows" ref={box} onScroll={onScroll} role="listbox" aria-label="Thread" tabIndex={-1}
      onMouseMove={onMouseMove} onMouseLeave={() => { pointer.current.inside = false; }}
      onWheel={() => { pointer.current.wheel = Date.now(); }}>
      <div className="cards-inner" style={{ height: layout.height + (rows.length < total ? 48 : 90) }}>
        {layout.heads
          .filter((h) => h.top + HEAD_H >= scroll - margin && h.top <= scroll + height + margin)
          .map((h) => (
            <div key={h.key} className="group-head" style={{ top: h.top, height: h.top === TOP ? HEAD_H - 10 : HEAD_H }}>{h.label}</div>
          ))}
        {ghosts.map((g) => (
          <Row key={`g${g.key}`} ghost t={g.t} index={-1} top={g.top} me={me} recent={g.recent}
            selected={false} picked={false} onOpen={noop} onDone={noop} onSnooze={noop} onTrash={noop} />
        ))}
        {items}
        {rows.length < total && <div className="list-foot" style={{ top: layout.height }}>Carico altri thread…</div>}
      </div>
    </div>
  );
}

const noop = () => {};

type RowProps = {
  t: ThreadSummary;
  index: number;
  top: number;
  me: string;
  /** Nei gruppi Oggi e Ieri si mostra solo l'ora. */
  recent: boolean;
  selected: boolean;
  picked: boolean;
  ghost?: boolean;
  onOpen: (i: number) => void;
  onDone: (i: number) => void;
  onSnooze: (i: number) => void;
  onTrash: (i: number) => void;
};

const Row = memo(function Row({ t, index, top, me, recent, selected, picked, ghost, onOpen, onDone, onSnooze, onTrash }: RowProps) {
  const cls = `card row${t.unread ? "" : " read"}${selected ? " selected" : ""}${picked ? " picked" : ""}${ghost ? " ghost" : ""}`;
  const files = t.attachment_names ?? [];
  const nFiles = Math.max(t.attachment_count ?? 0, files.length);
  const when = recent ? whenLabel(t.last_date).replace(/^(Oggi|Ieri) /, "") : whenLabel(t.last_date);
  return (
    <div className={cls} style={{ top, height: ROW_H }} role="option" aria-selected={selected} data-i={index} onClick={() => onOpen(index)}>
      <span className="r-dot">{t.unread && <i className="unread-dot" title="Da leggere" />}</span>
      <span className="r-who">
        <span className="nm">{participantsLabel(t.participants, me)}</span>
        {t.message_count > 1 && <span className="r-count">{t.message_count}</span>}
      </span>
      <span className="r-main">
        <span className="r-subj">{t.subject || "(nessun oggetto)"}</span>
        <span className="r-snip">{t.snippet}</span>
      </span>
      <span className="r-meta">
        {t.reminded && <span className="pill remind"><Alarm size={13} /> Promemoria</span>}
        {t.has_draft && <span className="r-draft">Bozza</span>}
        {t.starred && <Star size={15} filled style={{ color: "var(--accent)" }} />}
        {(nFiles > 0 || t.has_attachments) && (
          <span className="r-clip" title={files.length ? files.join(", ") + (nFiles > files.length ? ` e altri ${nFiles - files.length}` : "") : "Allegati"}>
            <Clip size={16} />
          </span>
        )}
        <span className="r-when">{when}</span>
        <span className="acts" onClick={(e) => e.stopPropagation()}>
          {t.in_inbox && (
            <button title="Fatto (E)" onClick={() => onDone(index)}>
              <Check size={19} />
            </button>
          )}
          {t.in_inbox && (
            <button title="Promemoria: posticipa (H)" onClick={() => onSnooze(index)}>
              <Alarm size={18} />
            </button>
          )}
          <button title="Cestina (#)" onClick={() => onTrash(index)}>
            <Trash size={18} />
          </button>
        </span>
      </span>
    </div>
  );
});
