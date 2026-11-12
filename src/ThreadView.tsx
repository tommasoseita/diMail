// Thread come conversazione (riferimento di Tommaso, 8 novembre): un messaggio dopo l'altro, con
// avatar, nome e ora, il testo pulito senza citazioni né firma. Le newsletter e le email solo HTML
// si vedono com'erano (in un riquadro chiaro); gli allegati come schede. In fondo la casella per
// rispondere (R), rispondere a tutti (A) o inoltrare (F).
// Sera del 9 novembre, come una chat: si apre già sull'ultimo messaggio, e i miei stanno a destra in una
// nuvoletta colorata, quelli degli altri a sinistra con l'avatar.
import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Addr, MessageView, ReplyMode, ThreadDetail, ThreadSummary } from "./api";
import { EmailFrame, inspectHtml } from "./EmailFrame";
import { addrLabel, avatarColor, bytes, chatTime, initials } from "./format";
import { inlineImagesOf, shownInline } from "./inlineImages";
import { ChevronDown, Download, FileIcon, Forward, Reply, ReplyAll, Search } from "./icons";
import { Keys } from "./Palette";

type Props = {
  detail: ThreadDetail | null;
  summary: ThreadSummary | undefined;
  me: string;
  /** Non più usato dalla vista a chat (i miei messaggi non hanno l'avatar). */
  myName?: string | null;
  labelName: (id: string) => string | null;
  contactOn: boolean;
  /** Il composer della risposta, se aperto su questo thread. */
  reply: React.ReactNode | null;
  onReply: (mode: ReplyMode) => void;
  /** Apre un allegato (lo scarica da Gmail la prima volta). */
  onOpenAttachment: (messageId: string, partId: string, name: string) => Promise<void>;
  onSaveAttachment: (messageId: string, partId: string, name: string) => Promise<void>;
  onPalette: () => void;
  onSearch: () => void;
  onToggleContact: () => void;
};

/**
 * Attesa massima perché i riquadri HTML abbiano la loro misura prima di mostrare la conversazione:
 * un'immagine lenta non blocca la vista.
 */
const READY_CAP = 350;

const HIDDEN_LABELS = new Set(["UNREAD", "SENT", "IMPORTANT", "INBOX", "STARRED", "DRAFT"]);

/** Nel testo semplice: la risposta e, a parte, la parte citata ("Il … ha scritto:" e righe con ">"). */
export function splitQuoted(text: string): [string, string | null] {
  const lines = text.split("\n");
  const header = /^(on .+ wrote:|il .+ ha scritto:|le .+ a écrit :|am .+ schrieb .+:|el .+ escribió:|-{2,} ?(original message|messaggio originale) ?-{2,}|from: .+|da: .+)$/i;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i].trim();
    if (header.test(l) || (l.startsWith(">") && lines.slice(i).filter((x) => x.trim()).every((x) => x.trim().startsWith(">")))) {
      const before = lines.slice(0, i).join("\n").trimEnd();
      if (before) return [before, lines.slice(i).join("\n")];
    }
  }
  return [text, null];
}

export function ThreadView({ detail, summary, me, labelName, contactOn, reply, onReply, onOpenAttachment, onSaveAttachment, onPalette, onSearch, onToggleContact }: Props) {
  const subject = detail?.thread.subject || summary?.subject || "";
  const labels = (detail?.thread.labels ?? summary?.labels ?? [])
    .filter((l) => !HIDDEN_LABELS.has(l) && !l.startsWith("CATEGORY_"))
    .map((l) => labelName(l))
    .filter((n): n is string => !!n);
  const last = detail?.messages[detail.messages.length - 1];
  const replyTo = last ? (last.from?.email === me ? last.to[0] : last.from) : null;
  const others = last ? new Set([last.from, ...last.to, ...last.cc].filter((a) => a && a.email !== me).map((a) => a!.email)).size : 0;
  const n = detail?.messages.length ?? 0;
  const scroller = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLDivElement>(null);

  // 11 novembre: la conversazione si mostra una volta sola, già composta e già sull'ultimo messaggio.
  // Finché i riquadri HTML non hanno la loro misura (al massimo READY_CAP) resta impaginata ma
  // invisibile; poi la posizione si fissa e appare con una breve dissolvenza. Dopo, se qualcosa sopra
  // cresce (un'immagine arrivata tardi), l'ultimo messaggio resta fermo dov'è, finché non scorro io.
  const [shown, setShown] = useState(false);
  const shownRef = useRef(false);
  const frameReady = useRef<() => void>(() => {});
  const onFrameReady = useCallback(() => frameReady.current(), []);
  const userInput = useRef<() => void>(() => {});
  const onUserInput = useCallback(() => userInput.current(), []);
  const threadId = detail?.thread.id;

  useLayoutEffect(() => {
    const box = scroller.current;
    const content = inner.current;
    if (!box || !content || !threadId) return;
    const reveal = () => {
      if (shownRef.current) return;
      shownRef.current = true;
      content.classList.remove("settling"); // subito, nello stesso fotogramma della posizione
      setShown(true);
    };
    if (n === 0) return reveal();
    let placed = false;
    let mine = false;
    let bottom = false;
    let at = 0;
    // Dove ho messo io la vista, e con quali misure: uno scorrimento diverso a misure uguali è dell'utente
    // (con misure cambiate è il browser che riporta la posizione nei limiti).
    let expected = 0;
    let size = "";
    const setTop = (v: number) => {
      box.scrollTop = v;
      expected = box.scrollTop;
      size = `${box.scrollHeight}x${box.clientHeight}`;
    };
    const lastMsg = () => {
      const msgs = content.querySelectorAll<HTMLElement>(".msg");
      return msgs[msgs.length - 1] ?? null;
    };
    // L'ultimo messaggio: in fondo se ci sta, altrimenti dal suo inizio.
    const place = () => {
      const last = lastMsg();
      if (!last) return;
      const end = box.scrollHeight - box.clientHeight;
      const top = last.offsetTop - 18;
      bottom = end <= top;
      setTop(Math.min(top, end));
      at = last.offsetTop - box.scrollTop;
      placed = true;
    };
    const unsettled = () => !!content.querySelector("iframe.email-frame:not([data-fit='ok'])");
    const settle = (force: boolean) => {
      if (placed || (!force && unsettled())) return;
      place();
      reveal();
    };
    const follow = () => {
      if (!placed) return settle(false);
      if (mine) return;
      const last = lastMsg();
      if (!last) return;
      if (bottom) place();
      else setTop(last.offsetTop - at);
    };
    frameReady.current = () => settle(false);
    const cap = setTimeout(() => settle(true), READY_CAP);
    settle(false); // solo testo semplice: si mostra prima del primo disegno
    const ro = new ResizeObserver(follow);
    ro.observe(content);
    ro.observe(box);
    const stop = () => {
      if (placed) mine = true;
    };
    const onScroll = () => {
      if (placed && Math.abs(box.scrollTop - expected) > 1 && size === `${box.scrollHeight}x${box.clientHeight}`) mine = true;
    };
    // Anche dentro le email (i riquadri hanno il loro documento: gli eventi non arrivano qui).
    userInput.current = stop;
    const events = ["wheel", "mousedown", "keydown", "touchstart"] as const;
    events.forEach((e) => box.addEventListener(e, stop, { passive: true }));
    box.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      ro.disconnect();
      clearTimeout(cap);
      frameReady.current = () => {};
      userInput.current = () => {};
      events.forEach((e) => box.removeEventListener(e, stop));
      box.removeEventListener("scroll", onScroll);
    };
  }, [threadId, n]);

  // "Apro il thread…" solo se ci vuole davvero: le aperture veloci non lo mostrano mai.
  const [slow, setSlow] = useState(false);
  const waiting = !detail;
  useEffect(() => {
    if (!waiting) return;
    const t = setTimeout(() => setSlow(true), 400);
    return () => clearTimeout(t);
  }, [waiting]);

  return (
    <div className="convo">
      <div className="convo-head">
        <button className="chip-btn" onClick={onPalette} title="Tutti i comandi">
          Palette dei comandi <span className="k"><Keys keys={["mod+k"]} /></span>
        </button>
        <span className="grow" />
        <button className="icon-btn" onClick={onSearch} title="Cerca nella posta (/)">
          <Search />
        </button>
        <button className={`scope-pill${contactOn ? " on" : ""}`} onClick={onToggleContact} title="Barra del contatto (I)">
          Contatto
        </button>
      </div>

      <div className="convo-scroll" tabIndex={-1} ref={scroller}>
        <div className={shown ? "convo-inner" : "convo-inner settling"} ref={inner}>
          <div className="convo-title">{subject || "(nessun oggetto)"}</div>
          {labels.length > 0 && <div className="convo-labels">{labels.map((l) => <span key={l} className="pill">{l}</span>)}</div>}
          {detail?.messages.map((m) => (
            <ChatMessage key={m.id} m={m} me={me} onFrameReady={onFrameReady} onUserInput={onUserInput}
              onOpenAttachment={onOpenAttachment} onSaveAttachment={onSaveAttachment} />
          ))}
        </div>
        {waiting && slow && <p className="pending convo-wait">Apro il thread…</p>}
      </div>

      <div className="composer">
        {reply ?? (
          <div className={detail ? "composer-box" : "composer-box waiting"} role="button" tabIndex={-1} onClick={() => onReply("reply")}>
            <div className="ph">{replyTo ? `Rispondi a ${addrLabel(replyTo, me)}…` : "Rispondi…"}</div>
            <div className="row" onClick={(e) => e.stopPropagation()}>
              <button className="reply-act" onClick={() => onReply("reply")} title="Rispondi (R)">
                <Reply size={17} /> Rispondi <Keys keys={["r"]} />
              </button>
              {others > 1 && (
                <button className="reply-act" onClick={() => onReply("reply_all")} title="Rispondi a tutti (A)">
                  <ReplyAll size={17} /> A tutti <Keys keys={["a"]} />
                </button>
              )}
              <button className="reply-act" onClick={() => onReply("forward")} title="Inoltra (F)">
                <Forward size={17} /> Inoltra <Keys keys={["f"]} />
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function ChatMessage({ m, me, onFrameReady, onUserInput, onOpenAttachment, onSaveAttachment }: {
  m: MessageView;
  me: string;
  /** Un riquadro HTML ha la sua misura. */
  onFrameReady: () => void;
  /** Rotella, clic o tasti dentro un'email. */
  onUserInput: () => void;
  onOpenAttachment: Props["onOpenAttachment"];
  onSaveAttachment: Props["onSaveAttachment"];
}) {
  const mine = m.from?.email === me;
  const html = m.html ?? null;
  const clean = (m.text_clean ?? "").trim();
  // Newsletter: l'email originale. Il resto: l'HTML come testo della conversazione, o il testo semplice.
  const [original, setOriginal] = useState(!!html && m.bulk);
  // Passando tra conversazione ed email originale, la vista di prima resta finché la nuova non ha la
  // sua misura (App.css, .hold): niente fotogramma vuoto in mezzo.
  const [hold, setHold] = useState<null | "chat" | "original">(null);
  const toggleOriginal = () => {
    setHold(original ? "original" : "chat");
    setOriginal((o) => !o);
  };
  const [quotes, setQuotes] = useState(false);
  // Senza citazioni non restava niente (un inoltro): le citazioni restano, senza il pulsante per toglierle.
  const [forced, setForced] = useState(false);
  const [signature, setSignature] = useState(false);
  const [images, setImages] = useState(false);
  const [details, setDetails] = useState(false);
  // Clic sull'intestazione: mittente, destinatari e data per esteso. Non se stavo selezionando il testo.
  const toggleDetails = () => {
    if (!window.getSelection()?.toString()) setDetails((x) => !x);
  };
  const info = useMemo(() => (html ? inspectHtml(html) : { hasQuotes: false, hasRemoteImages: false, hasSignature: false }), [html]);
  const [plainHead, plainQuoted] = useMemo(() => (m.text_plain ? splitQuoted(m.text_plain) : ["", null]), [m.text_plain]);
  const text = useMemo(() => tidyPlain(clean || plainHead.trim()), [clean, plainHead]);
  const hasMore = !!m.text_plain && m.text_plain.trim().length > text.length + 20;
  const allowRemote = m.trusted_sender || images;
  // Le immagini incollate nel testo si vedono nel testo, non tra gli allegati.
  const files = m.attachments.filter((a) => !shownInline(a, html));
  const inlineImage = useMemo(() => inlineImagesOf(m), [m.id, m.attachments]);
  const name = mine ? "Tu" : addrLabel(m.from, me);
  const recipients = [...m.to, ...m.cc].filter((a) => a.email !== me).map((a) => addrLabel(a, me));

  const col = useRef<HTMLDivElement>(null);
  /** Aperto senza corpo: il corpo arriva dopo, scaricato da Gmail. */
  const late = useRef(!m.has_body);
  // La nuvoletta di un messaggio HTML si stringe sul testo, fino al 92% della colonna (come nel CSS).
  const bubbleMax = () => Math.max(160, (col.current?.clientWidth ?? 600) * 0.92 - 34);

  return (
    <article className={mine ? "msg mine" : "msg"}>
      {!mine && (
        <div className="avatar" style={{ background: avatarColor(m.from?.email ?? "?") }}>
          {initials(m.from?.name, m.from?.email ?? "?")}
        </div>
      )}
      <div className="msg-col" ref={col}>
        <div className={details ? "msg-head open" : "msg-head"} role="button" tabIndex={-1} onClick={toggleDetails}
          title={details ? "Nascondi i dettagli" : "Mittente, destinatari e data"}>
          {!mine && <span className="msg-name">{name}</span>}
          {mine && recipients.length > 0 && <span className="msg-to">a {recipients.join(", ")}</span>}
          <span className="msg-time">{chatTime(m.date)}</span>
          <ChevronDown className="msg-more" size={14} />
        </div>
        {!mine && recipients.length > 1 && (
          <div className="msg-to" role="button" tabIndex={-1} onClick={toggleDetails}>a {recipients.join(", ")}</div>
        )}
        {details && <MessageDetails m={m} me={me} />}

        {/* Il corpo arrivato dopo: l'anteprima resta finché il riquadro non ha la sua misura (App.css). */}
        {(!m.has_body || (late.current && html && !original)) && (
          <div className={m.has_body ? "bubble snippet-hold" : "bubble"}><p className="pending">{m.snippet} <em>(scarico il messaggio…)</em></p></div>
        )}

        {m.has_body && !html && (
          <div className="bubble"><div className="msg-text">{quotes && m.text_plain ? m.text_plain.trim() : text || m.snippet}</div></div>
        )}
        {m.has_body && html && (!original || hold === "chat") && (
          <div className={original ? "bubble msg-html hold" : "bubble msg-html"}>
            <EmailFrame html={html} allowRemote={allowRemote} showQuotes={quotes} variant="chat" showSignature={signature}
              fitWidth={bubbleMax} inlineImage={inlineImage} onEmpty={() => { setForced(true); setQuotes(true); }} onUserInput={onUserInput}
              onReady={() => { late.current = false; onFrameReady(); setHold((h) => (h === "original" ? null : h)); }} />
          </div>
        )}
        {m.has_body && html && (original || hold === "original") && (
          <div className={original ? "embed html" : "embed html hold"}>
            <EmailFrame html={html} allowRemote={allowRemote} showQuotes={quotes} inlineImage={inlineImage} onUserInput={onUserInput}
              onReady={() => { onFrameReady(); setHold((h) => (h === "chat" ? null : h)); }} />
          </div>
        )}

        {files.map((a) => (
          <Attachment key={a.part_id} name={a.filename || "allegato senza nome"} size={a.size}
            onOpen={() => onOpenAttachment(m.id, a.part_id, a.filename)} onSave={() => onSaveAttachment(m.id, a.part_id, a.filename)} />
        ))}

        {m.has_body && (html || hasMore || plainQuoted) && (
          <div className="msg-tools">
            {(html ? info.hasQuotes && !forced : hasMore || plainQuoted) && (
              <button onClick={() => setQuotes((q) => !q)}>{quotes ? "Nascondi il testo citato" : "Testo citato"}</button>
            )}
            {html && !original && info.hasSignature && (
              <button onClick={() => setSignature((s) => !s)}>{signature ? "Nascondi la firma" : "Firma"}</button>
            )}
            {html && (
              <button onClick={toggleOriginal}>{original ? "Torna alla conversazione" : "Email originale"}</button>
            )}
            {info.hasRemoteImages && !allowRemote && (
              <button onClick={() => setImages(true)} title="Bloccate perché il mittente non è tra quelli a cui hai scritto">
                Mostra le immagini
              </button>
            )}
          </div>
        )}
      </div>
    </article>
  );
}

const fullDate = new Intl.DateTimeFormat("it-IT", {
  weekday: "long", day: "numeric", month: "long", year: "numeric", hour: "2-digit", minute: "2-digit",
});

/** Chi ha scritto a chi, per esteso: nomi e indirizzi, selezionabili per copiarli. */
function MessageDetails({ m, me }: { m: MessageView; me: string }) {
  const who = (a: Addr) => (
    <span className="who">
      {a.name && a.name !== a.email ? <>{a.email === me ? "io" : a.name} <span className="em">{a.email}</span></> : a.email}
    </span>
  );
  const row = (label: string, list: Addr[]) =>
    list.length > 0 && (
      <>
        <dt>{label}</dt>
        <dd>{list.map((a, i) => <Fragment key={a.email}>{i > 0 && ", "}{who(a)}</Fragment>)}</dd>
      </>
    );
  return (
    <dl className="msg-details">
      {row("Da", m.from ? [m.from] : [])}
      {row("A", m.to)}
      {row("Cc", m.cc)}
      <dt>Data</dt>
      <dd>{fullDate.format(new Date(m.date))}</dd>
      {m.subject && (
        <>
          <dt>Oggetto</dt>
          <dd>{m.subject}</dd>
        </>
      )}
    </dl>
  );
}

export function tidyPlain(text: string): string {
  return text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((l) => l.replace(/^(\s*)[*•-]\s+/, "$1• ").replace(/^\*([^*\n]+)\*$/, "$1"))
    .join("\n")
    // Gmail mette una riga vuota tra le voci di un elenco: si toglie.
    .replace(/(^|\n)(• [^\n]*)\n\n(?=• )/g, "$1$2\n")
    .replace(/(\n• [^\n]*)\n\n(?=• )/g, "$1\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** Scheda di un allegato: clic per aprirlo, la freccia per salvarlo in Download. */
function Attachment({ name, size, onOpen, onSave }: { name: string; size: number; onOpen: () => Promise<void>; onSave: () => Promise<void> }) {
  const [busy, setBusy] = useState<null | "open" | "save">(null);
  const run = async (what: "open" | "save", f: () => Promise<void>) => {
    if (busy) return;
    setBusy(what);
    try {
      await f();
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className={busy ? "attach-card busy" : "attach-card"} role="button" tabIndex={-1} title={`Apri ${name}`}
      onClick={() => run("open", onOpen)}>
      <FileIcon size={20} />
      <span className="nm">{name}</span>
      <span className="pill">{busy === "open" ? "Apro…" : busy === "save" ? "Salvo…" : bytes(size)}</span>
      <button className="attach-save" title="Salva in Download" onClick={(e) => { e.stopPropagation(); run("save", onSave); }}>
        <Download size={18} />
      </button>
    </div>
  );
}
