// Composer di base (9 novembre): nuova email, rispondi, rispondi a tutti, inoltra. Cmd+Invio invia,
// con 30 secondi per annullare (li gestisce Workspace). Cmd+J chiede all'AI di scrivere la risposta,
// o di sistemare quello che ho scritto (appunti, bozza grezza); "Ripristina" torna al mio testo.
// Sera del 9 novembre: allegati (graffetta o file trascinati dal Finder; negli inoltri quelli del
// messaggio originale), la firma di Gmail sotto il testo, e il pannello che si ingrandisce su tutta
// l'area della posta (barra laterale esclusa), allargandosi dal suo posto.
// Le bozze di Gmail e gli snippet arrivano dopo.
import { useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { api, type Addr, type Draft, type OutAttachment, type ReplyMode, type Signature } from "./api";
import { BodyEditor, type BodyEditorHandle } from "./BodyEditor";
import { EmailFrame } from "./EmailFrame";
import { useFileDrop } from "./fileDrop";
import { addrLabel, bytes } from "./format";
import { Clip, Close, Expand, FileIcon, Send, Shrink, Sparkle } from "./icons";
import { Keys } from "./Palette";

export type ComposeMode = "new" | ReplyMode;

type Props = {
  initial: Draft;
  mode: ComposeMode;
  inline?: boolean;
  me: string;
  onSend: (d: Draft) => void;
  /** Chiuso senza inviare: la bozza corrente, per riaprirla. */
  onClose: (d: Draft) => void;
  /** Cmd+J: il testo scritto dall'AI a partire dalla bozza. */
  onAiWrite: (d: Draft, mode: ComposeMode) => Promise<string>;
};

const EMAIL = /^[^\s@<>(),;:"]+@[^\s@<>(),;:"]+\.[^\s@<>(),;:"]+$/;
/** Il limite di Gmail per gli allegati di un'email. */
const MAX_FILES = 25 * 1024 * 1024;

/** "Anna <anna@x.it>", "anna@x.it", "anna@x.it, luca@y.it" → indirizzi. */
function parseAddrs(text: string): Addr[] {
  return text
    .split(/[,;\n]+/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => {
      const m = s.match(/^(.*)<([^>]+)>$/);
      if (m) return { name: m[1].trim().replace(/^"|"$/g, "") || undefined, email: m[2].trim() };
      return { email: s };
    })
    .filter((a) => EMAIL.test(a.email));
}

const TITLES: Record<ComposeMode, string> = {
  new: "Nuovo messaggio",
  reply: "Rispondi",
  reply_all: "Rispondi a tutti",
  forward: "Inoltra",
};

const fileKey = (a: OutAttachment) => a.path ?? `${a.message_id}/${a.part_id}`;

export function Composer({ initial, mode, inline, me, onSend, onClose, onAiWrite }: Props) {
  const [d, setD] = useState<Draft>(() => ({ ...initial, attachments: initial.attachments ?? [] }));
  const [showCc, setShowCc] = useState(initial.cc.length > 0 || initial.bcc.length > 0);
  const [showQuoted, setShowQuoted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  /** Il mio testo prima dell'AI, per ripristinarlo. */
  const [before, setBefore] = useState<string | null>(null);
  const [signature, setSignature] = useState<Signature | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [dragging, setDragging] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const body = useRef<BodyEditorHandle>(null);
  const toRef = useRef<HTMLInputElement>(null);
  /** Dopo ⌘J il cursore torna in fondo al testo, quando il campo è di nuovo modificabile. */
  const focusAfterAi = useRef(false);
  const latest = useRef(d);
  latest.current = d;

  // Il cursore va dove si scrive: nel corpo per le risposte, nel "A" per il resto.
  useEffect(() => {
    if (mode === "reply" || mode === "reply_all") body.current?.focus();
    else toRef.current?.focus();
  }, [mode]);

  // La firma di Gmail web (riletta da Gmail al massimo una volta l'ora).
  useEffect(() => {
    let alive = true;
    api.signature().then((s) => alive && setSignature(s)).catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const addFiles = (files: OutAttachment[]) => {
    if (!files.length) return;
    setError(null);
    setD((x) => {
      const seen = new Set(x.attachments.map(fileKey));
      return { ...x, attachments: [...x.attachments, ...files.filter((f) => !seen.has(fileKey(f)))] };
    });
  };

  const pickFiles = async () => {
    try {
      addFiles(await api.pickAttachments());
    } catch (e) {
      setError(String(e));
    }
  };

  // File trascinati dal Finder: si allegano qui.
  useFileDrop(true, {
    over: setDragging,
    drop: (paths) => api.fileInfo(paths).then(addFiles, (e) => setError(String(e))),
  });

  const filesSize = d.attachments.reduce((n, a) => n + (a.size || 0), 0);

  const send = () => {
    const pending = toRef.current?.value ? parseAddrs(toRef.current.value) : [];
    const draft = { ...latest.current, to: [...latest.current.to, ...pending] };
    if (draft.to.length + draft.cc.length + draft.bcc.length === 0) {
      setError("Manca il destinatario.");
      toRef.current?.focus();
      return;
    }
    if (!draft.body.trim() && !draft.quoted && !draft.attachments.length) {
      setError("Il messaggio è vuoto.");
      body.current?.focus();
      return;
    }
    if (filesSize > MAX_FILES) {
      setError("Gli allegati superano 25 MB, il limite di Gmail: mandali con un link di Drive.");
      return;
    }
    onSend(draft);
  };

  const aiWrite = async () => {
    if (writing) return;
    const cur = latest.current;
    setWriting(true);
    setError(null);
    try {
      const text = await onAiWrite(cur, mode);
      setBefore(cur.body);
      setD((x) => ({ ...x, body: text }));
      focusAfterAi.current = true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setWriting(false);
    }
  };

  useEffect(() => {
    if (writing || !focusAfterAi.current) return;
    focusAfterAi.current = false;
    body.current?.focusEnd();
  }, [writing]);

  // Ingrandisci e rimpicciolisci: il pannello passa da un rettangolo all'altro (FLIP sulle sue
  // dimensioni vere, non una scala: il testo non si deforma), sopra lista e conversazione.
  const toggleExpand = () => {
    const el = root.current;
    const parent = el?.offsetParent as HTMLElement | null;
    if (!el || !parent) {
      setExpanded((x) => !x);
      return;
    }
    const from = el.getBoundingClientRect();
    flushSync(() => setExpanded((x) => !x));
    const to = el.getBoundingClientRect();
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const p = parent.getBoundingClientRect();
    const ox = p.left + parent.clientLeft;
    const oy = p.top + parent.clientTop;
    const frame = (r: DOMRect) => ({
      top: `${r.top - oy}px`, left: `${r.left - ox}px`, width: `${r.width}px`, height: `${r.height}px`, maxHeight: "none",
    });
    el.animate([frame(from), frame(to)], { duration: 380, easing: "cubic-bezier(.22, .9, .24, 1)" });
    body.current?.focus();
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key.toLowerCase() === "j" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopPropagation();
      aiWrite();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopPropagation();
      send();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      // Ingrandito, Esc lo riporta piccolo; poi chiude.
      if (expanded) toggleExpand();
      else onClose(latest.current);
    }
  };

  const set = (patch: Partial<Draft>) => {
    setError(null);
    setD((x) => ({ ...x, ...patch }));
  };

  const cls = `${inline ? "compose inline" : "compose panel-c"}${expanded ? " expanded" : ""}${dragging ? " dragging" : ""}`;
  const sig = signature && !d.no_signature ? signature : null;

  return (
    <>
      {!inline && <div className={expanded ? "compose-backdrop on" : "compose-backdrop"} onClick={() => expanded && toggleExpand()} />}
      <div ref={root} className={cls} onKeyDown={onKey} role="dialog" aria-label={TITLES[mode]}>
        {!inline && (
          <div className="compose-head">
            <span className="t">{TITLES[mode]}</span>
            <span className="grow" />
            <button className="icon-btn" onClick={toggleExpand} title={expanded ? "Rimpicciolisci (Esc)" : "Ingrandisci"}>
              {expanded ? <Shrink size={17} /> : <Expand size={17} />}
            </button>
            <button className="icon-btn" onClick={() => onClose(latest.current)} title="Chiudi">
              <Close size={17} />
            </button>
          </div>
        )}
        <div className="compose-fields">
          <RecipientField label="A" value={d.to} me={me} inputRef={toRef} onChange={(to) => set({ to })}
            extra={!showCc && <button className="link-btn" onClick={() => setShowCc(true)}>Cc Ccn</button>} />
          {showCc && <RecipientField label="Cc" value={d.cc} me={me} onChange={(cc) => set({ cc })} />}
          {showCc && <RecipientField label="Ccn" value={d.bcc} me={me} onChange={(bcc) => set({ bcc })} />}
          {(!inline || mode === "forward") && (
            <label className="field">
              <span className="lbl">Oggetto</span>
              <input value={d.subject} onChange={(e) => set({ subject: e.target.value })} spellCheck />
            </label>
          )}
        </div>
        <div className="compose-scroll" onMouseDown={(e) => {
          // Un clic nel bianco sotto il testo scrive nel testo.
          if (e.target === e.currentTarget) {
            e.preventDefault();
            body.current?.focusEnd();
          }
        }}>
          <div className={writing ? "compose-body-wrap writing" : "compose-body-wrap"}>
            {/* Il corpo cresce con il testo; nel pannello ingrandito senza limite (scorre il pannello). */}
            <BodyEditor ref={body} className="compose-body" value={d.body} readOnly={writing} maxHeight={expanded ? undefined : inline ? 320 : 420}
              placeholder={inline ? "Scrivi la risposta, anche solo due appunti: ⌘J la sistema" : "Scrivi, anche solo due appunti: ⌘J li trasforma in un'email"}
              onChange={(text) => { set({ body: text }); if (before !== null) setBefore(null); }} />
            {writing && <div className="ai-writing"><Sparkle size={15} /> L'AI sta scrivendo…</div>}
          </div>
          {before !== null && !writing && (
            <div className="ai-done">
              <Sparkle size={14} /> Scritto dall'AI: rileggilo prima di inviare.
              <button className="link-btn" onClick={() => { setD((x) => ({ ...x, body: before })); setBefore(null); }}>Ripristina il mio testo</button>
              <button className="link-btn" onClick={aiWrite}>Riscrivi</button>
            </div>
          )}
          {sig && (
            <div className="compose-sig" title="La tua firma di Gmail">
              <div className="sig-frame">
                <EmailFrame html={sig.html} allowRemote showQuotes variant="chat" />
              </div>
              <button className="sig-x" onClick={() => set({ no_signature: true })} title="Togli la firma da questa email" aria-label="Togli la firma">
                <Close size={14} />
              </button>
            </div>
          )}
          {signature && d.no_signature && (
            <button className="link-btn sig-add" onClick={() => set({ no_signature: false })}>Aggiungi la firma</button>
          )}
          {d.quoted && (
            <div className="quoted">
              <button className="link-btn" onClick={() => setShowQuoted((q) => !q)}>
                {showQuoted ? "Nascondi il testo citato" : mode === "forward" ? "Messaggio inoltrato ···" : "Testo citato ···"}
              </button>
              {showQuoted && <pre>{d.quoted}</pre>}
            </div>
          )}
          {d.attachments.length > 0 && (
            <div className="compose-files">
              {d.attachments.map((a) => (
                <span key={fileKey(a)} className="file-chip" title={a.name}>
                  <FileIcon size={16} />
                  <span className="nm">{a.name}</span>
                  {a.size > 0 && <span className="sz">{bytes(a.size)}</span>}
                  <button onClick={() => set({ attachments: d.attachments.filter((x) => fileKey(x) !== fileKey(a)) })} aria-label={`Togli ${a.name}`} title="Togli">
                    <Close size={13} />
                  </button>
                </span>
              ))}
              {filesSize > MAX_FILES && <span className="compose-err">Oltre 25 MB: Gmail non li invia.</span>}
            </div>
          )}
        </div>
        <div className="compose-foot">
          <button className="send-btn" onClick={send} title="Invia (⌘↩)">
            <Send size={16} /> Invia <Keys keys={["mod+enter"]} />
          </button>
          <button className="ai-btn" onClick={aiWrite} disabled={writing}
            title={d.body.trim() ? "L'AI sistema quello che hai scritto (⌘J)" : "L'AI scrive la risposta (⌘J)"}>
            <Sparkle size={15} /> {d.body.trim() ? "Sistema con l'AI" : "Scrivi con l'AI"} <Keys keys={["mod+j"]} />
          </button>
          <button className="icon-btn attach-btn" onClick={pickFiles} title="Allega file (o trascinali qui)">
            <Clip size={18} />
          </button>
          {error && <span className="compose-err">{error}</span>}
          <span className="grow" />
          {inline && (
            <button className="icon-btn" onClick={() => onClose(latest.current)} title="Chiudi (Esc)">
              <Close size={17} />
            </button>
          )}
        </div>
        {dragging && <div className="drop-hint"><Clip size={20} /> Rilascia per allegare</div>}
      </div>
    </>
  );
}

function RecipientField({ label, value, me, onChange, inputRef, extra }: {
  label: string;
  value: Addr[];
  me: string;
  onChange: (v: Addr[]) => void;
  inputRef?: React.RefObject<HTMLInputElement | null>;
  extra?: React.ReactNode;
}) {
  const [text, setText] = useState("");
  const [hints, setHints] = useState<Addr[]>([]);
  const [active, setActive] = useState(0);
  const own = useRef<HTMLInputElement>(null);
  const ref = inputRef ?? own;

  useEffect(() => {
    const q = text.trim();
    if (q.length < 1) {
      setHints([]);
      return;
    }
    let alive = true;
    const t = setTimeout(() => {
      api.suggestContacts(q).then((h) => {
        if (!alive) return;
        setHints(h.filter((a) => !value.some((v) => v.email.toLowerCase() === a.email.toLowerCase())));
        setActive(0);
      }).catch(() => {});
    }, 100);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [text, value]);

  const add = (a: Addr[]) => {
    if (!a.length) return;
    const seen = new Set(value.map((v) => v.email.toLowerCase()));
    onChange([...value, ...a.filter((x) => !seen.has(x.email.toLowerCase()))]);
    setText("");
    setHints([]);
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (hints.length && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      setActive((i) => Math.max(0, Math.min(hints.length - 1, i + (e.key === "ArrowDown" ? 1 : -1))));
    } else if ((e.key === "Enter" || e.key === "Tab" || e.key === ",") && !e.metaKey && !e.ctrlKey) {
      if (hints.length && text.trim()) {
        e.preventDefault();
        add([hints[active]]);
      } else if (text.trim()) {
        const parsed = parseAddrs(text);
        if (parsed.length) {
          e.preventDefault();
          add(parsed);
        }
      }
    } else if (e.key === "Backspace" && !text && value.length) {
      onChange(value.slice(0, -1));
    }
  };

  return (
    <div className="field" onClick={() => ref.current?.focus()}>
      <span className="lbl">{label}</span>
      <div className="chips">
        {value.map((a) => (
          <span key={a.email} className="chip" title={a.email}>
            {addrLabel(a, me)}
            <button onClick={(e) => { e.stopPropagation(); onChange(value.filter((x) => x.email !== a.email)); }} aria-label="Togli">×</button>
          </span>
        ))}
        <input ref={ref} value={text} onChange={(e) => setText(e.target.value)} onKeyDown={onKey} spellCheck={false}
          onBlur={() => { const p = parseAddrs(text); if (p.length) add(p); setTimeout(() => setHints([]), 120); }}
          onPaste={(e) => {
            const p = parseAddrs(e.clipboardData.getData("text"));
            if (p.length > 1) {
              e.preventDefault();
              add(p);
            }
          }} />
      </div>
      {extra}
      {hints.length > 0 && (
        <ul className="hints" role="listbox">
          {hints.map((h, i) => (
            <li key={h.email} className={i === active ? "active" : ""} onMouseDown={(e) => { e.preventDefault(); add([h]); }}
              onMouseMove={() => setActive(i)}>
              <span className="nm">{h.name || h.email}</span>
              {h.name && <span className="em">{h.email}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
