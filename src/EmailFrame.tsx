import { openUrl } from "@tauri-apps/plugin-opener";
import { useLayoutEffect, useRef } from "react";

// L'HTML della email, già ripulito dal nucleo, in un iframe isolato: niente script (sandbox senza
// allow-scripts) e una CSP restrittiva. Le immagini remote si caricano solo per i mittenti conosciuti
// o su richiesta (sez. 6 della spec), anche contro i pixel di tracciamento.
//
// Due modi. "chat" (9 novembre): il messaggio come testo della conversazione, con il carattere e i
// colori di diMail, elenchi e grassetti veri, senza firma né testo citato (si aprono a richiesta).
// "original": l'email com'era, su fondo chiaro anche nel tema scuro (newsletter e su richiesta).

export const SIGNATURES =
  ".gmail_signature,[data-smartmail=gmail_signature],.gmail_signature_prefix,#Signature,#AppleMailSignature,.moz-signature,#ms-outlook-mobile-signature,div[id^=signature i],div[class^=signature i]";

const QUOTES = ".gmail_quote,blockquote[type=cite],.yahoo_quoted,#divRplyFwdMsg,#divRplyFwdMsg~*,.moz-cite-prefix,.moz-cite-prefix~blockquote,#appendonsend~*";

// Le risposte di Outlook non segnano la citazione con una classe: la riconosce l'intestazione del
// messaggio citato, "From: … Sent: … To: … Subject: …" (o "Da: … Inviato: … A: … Oggetto: …"),
// spesso sotto una riga sottile. Da lì in fondo è tutto citato (11 novembre).
const HEAD_FROM = /^(from|da|von|de|van|från|fra|od)\s*:/i;
const HEAD_REST = /(sent|inviato|gesendet|envoy[ée]|enviado|verzonden|skickat|sendt|date|data|datum|fecha)\s*:[\s\S]{1,500}?(subject|oggetto|betreff|objet|asunto|onderwerp|assunto|[äa]mne|emne)\s*:/i;
const HEAD_IN_HTML = new RegExp(
  `(^|\\s)(from|da|von|de|van|från|fra|od)\\s*:\\s[\\s\\S]{1,300}?${HEAD_REST.source}`,
  "i",
);

function outlookQuote(html: string): boolean {
  if (!/(subject|oggetto|betreff|objet|asunto|onderwerp|assunto|[äa]mne|emne)\s*:/i.test(html)) return false;
  const text = html.replace(/<[^>]*>/g, " ").replace(/&nbsp;|&#160;|\u00a0/g, " ");
  return HEAD_IN_HTML.test(text);
}

/** Nasconde l'intestazione di risposta di Outlook e tutto quello che segue. Vero se l'ha trovata. */
function hideHeaderQuote(doc: Document): boolean {
  const body = doc.body;
  const walker = doc.createTreeWalker(body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!HEAD_FROM.test((n.textContent ?? "").replace(/\u00a0/g, " ").trim())) continue;
    // Il testo che segue deve avere la data e l'oggetto, a poca distanza.
    let after = "";
    const ahead = doc.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    ahead.currentNode = n;
    for (let m: Node | null = n; m && after.length < 900; m = ahead.nextNode()) after += ` ${m.textContent ?? ""}`;
    if (!HEAD_REST.test(after.replace(/\u00a0/g, " "))) continue;
    // Si risale finché prima, nello stesso contenitore, non c'è altro testo (la riga sottile di
    // Outlook è il bordo di un div che contiene solo l'intestazione).
    let start = n.parentElement;
    if (!start) return false;
    while (start.parentElement && start.parentElement !== body) {
      const range = doc.createRange();
      range.setStart(start.parentElement, 0);
      range.setEndBefore(start);
      if (range.toString().replace(/\u00a0/g, " ").trim()) break;
      start = start.parentElement;
    }
    const hide = (el: Element) => (el as HTMLElement).style.setProperty("display", "none", "important");
    const prev = start.previousElementSibling;
    if (prev && (prev.tagName === "HR" || (prev.querySelector("hr") && !prev.textContent?.trim()))) hide(prev);
    hide(start);
    for (let e: Element | null = start; e && e !== body; e = e.parentElement) {
      for (let sib = e.nextSibling; sib; sib = sib.nextSibling) {
        if (sib.nodeType === Node.ELEMENT_NODE) hide(sib as Element);
        else if (sib.nodeType === Node.TEXT_NODE && sib.textContent?.trim()) {
          const wrap = doc.createElement("span");
          sib.parentNode!.insertBefore(wrap, sib);
          wrap.appendChild(sib);
          hide(wrap);
          sib = wrap;
        }
      }
    }
    return true;
  }
  return false;
}

export function inspectHtml(html: string): { hasQuotes: boolean; hasRemoteImages: boolean; hasSignature: boolean } {
  return {
    hasQuotes: /gmail_quote|type="?cite|yahoo_quoted|divRplyFwdMsg|moz-cite-prefix|appendonsend/i.test(html) || outlookQuote(html),
    hasSignature: /gmail_signature|id="?(Signature|AppleMailSignature|ms-outlook-mobile-signature)|moz-signature|(id|class)="?signature/i.test(html),
    hasRemoteImages: /<img[^>]+src="?https?:/i.test(html) || /url\(\s*['"]?https?:/i.test(html),
  };
}

export type Variant = "chat" | "original";

/** I colori del tema corrente, per il modo "chat". */
function themeColors() {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return { text: v("--text", "#1c1c1a"), muted: v("--muted", "#8b8b87"), link: v("--link", "#2d6fd6"), line: v("--line-2", "#dbdbd8") };
}

function chatStyle(showQuotes: boolean, showSignature: boolean): string {
  const c = themeColors();
  return `
  html,body{margin:0;padding:0;background:transparent;color:${c.text}}
  body{font:16px/1.62 -apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",sans-serif;overflow-wrap:anywhere;padding:0}
  body *:not(a){color:inherit!important;background:transparent!important;font-family:inherit!important;font-size:inherit!important;line-height:inherit!important}
  body *{max-width:100%!important;box-sizing:border-box}
  a,a *{color:${c.link}!important;text-decoration:none}
  a:hover{text-decoration:underline}
  p{margin:0 0 .75em}
  div:empty{display:none}
  ul,ol{margin:.35em 0 .8em;padding-left:1.35em}
  li{margin:.3em 0;padding-left:.15em}
  li::marker{color:${c.muted}}
  h1,h2,h3,h4{font-weight:650!important;margin:.6em 0 .4em}
  table{border-collapse:collapse;width:auto!important}
  td,th{padding:4px 8px;border:1px solid ${c.line}!important;vertical-align:top}
  table[role=presentation] td,table:not([border]) td{border:0!important;padding:0 6px 0 0}
  img{max-width:100%;height:auto}
  hr{border:0;border-top:1px solid ${c.line};margin:1em 0}
  blockquote{margin:.4em 0;padding-left:.9em;border-left:2px solid ${c.line};color:${c.muted}!important}
  blockquote *{color:${c.muted}!important}
  ${SIGNATURES}{color:${c.muted}!important;font-size:14px!important;margin-top:.8em}
  ${SIGNATURES} *{color:${c.muted}!important;font-size:14px!important}
  ${showQuotes ? "" : `${QUOTES}{display:none!important}`}
  ${showSignature ? "" : `${SIGNATURES}{display:none!important}`}
  body>div:last-child>br:last-child,body>br:last-child{display:none}`;
}

function srcdoc(html: string, allowRemote: boolean, showQuotes: boolean, variant: Variant, showSignature: boolean): string {
  const img = allowRemote ? "data: https: http:" : "data:";
  const csp = `default-src 'none'; img-src ${img}; style-src 'unsafe-inline'; font-src data:`;
  if (variant === "chat") {
    return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<style>${chatStyle(showQuotes, showSignature)}</style></head><body>${html}</body></html>`;
  }
  return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<style>
  html,body{margin:0;padding:0;background:#fff;color:#1d1d1f}
  body{font:14px/1.55 -apple-system,BlinkMacSystemFont,"Helvetica Neue",sans-serif;overflow-wrap:anywhere;padding:12px 14px}
  img{max-width:100%;height:auto}
  table{max-width:100%}
  a{color:#2d7dd7}
  blockquote{margin:0 0 0 .6em;padding-left:.8em;border-left:2px solid #d5dde0;color:#5b6d72}
  ${showQuotes ? "" : `${QUOTES}{display:none!important}`}
</style></head><body>${html}</body></html>`;
}

/** Righe vuote in fondo (quelle che restavano prima della firma o della citazione nascoste): via. */
function trimTail(doc: Document) {
  const visible = (n: Node) => {
    const el = n.nodeType === Node.ELEMENT_NODE ? (n as Element) : n.parentElement;
    return !!el && el.getClientRects().length > 0;
  };
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  let last: Node | null = null;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const content = n.nodeType === Node.TEXT_NODE ? !!n.textContent?.trim() : (n as Element).tagName === "IMG";
    if (content && visible(n)) last = n;
  }
  if (!last) return;
  doc.body.querySelectorAll("br, div, p").forEach((el) => {
    if (!(last!.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING)) return;
    if (el.contains(last)) return;
    if (el.tagName === "BR" || (!(el as HTMLElement).innerText.trim() && !el.querySelector("img"))) (el as HTMLElement).style.display = "none";
  });
}

/** La larghezza del contenuto (righe di testo, immagini, tabelle), per stringere il riquadro su di esso. */
function contentWidth(doc: Document): number {
  let right = 0;
  const left = doc.body.getBoundingClientRect().left;
  const range = doc.createRange();
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) {
      if (!n.textContent?.trim()) continue;
      range.selectNodeContents(n);
      for (const r of range.getClientRects()) if (r.width > 0) right = Math.max(right, r.right);
    } else if (["IMG", "TABLE", "HR", "PRE"].includes((n as Element).tagName)) {
      const r = (n as Element).getBoundingClientRect();
      if (r.width > 0) right = Math.max(right, r.right);
    }
  }
  return Math.ceil(right - left);
}

/**
 * Immagini che prenderanno spazio solo quando arrivano: visibili, ancora in caricamento, senza
 * larghezza e altezza in numeri (con quelle il riquadro ha già la sua proporzione).
 */
function growingImages(doc: Document): boolean {
  const num = (v: string | null) => !!v && /^\d+$/.test(v.trim());
  return Array.from(doc.images).some(
    (img) => !img.complete && img.getClientRects().length > 0 && !(num(img.getAttribute("width")) && num(img.getAttribute("height"))),
  );
}

/** Per ogni riquadro, il documento che c'era quando gli si è dato l'ultimo srcdoc. */
const staleDocs = new WeakMap<HTMLIFrameElement, Document | null>();

// 11 novembre: niente più scatti all'apertura. Il documento si prepara (citazione di Outlook, righe
// vuote in fondo, corpo vuoto) e si misura appena è stato letto, senza aspettare `load`, che arriva
// solo dopo tutte le immagini, anche i loghi nascosti nella firma. Prima di quella misura il riquadro
// non si vede e non occupa spazio (App.css, `data-fit`); le immagini che arrivano dopo lo rimisurano.
export function EmailFrame({ html, allowRemote, showQuotes, variant = "original", showSignature = true, fitWidth, onEmpty, onReady, onUserInput }: {
  html: string;
  allowRemote: boolean;
  showQuotes: boolean;
  variant?: Variant;
  showSignature?: boolean;
  /** Senza citazioni e firma non resta testo (per esempio un inoltro): il chiamante mostra il resto. */
  onEmpty?: () => void;
  /** Il riquadro si stringe sul contenuto, fino a questa larghezza (le nuvolette della conversazione). */
  fitWidth?: () => number;
  /** Dopo ogni misura: la conversazione aspetta i riquadri prima di mostrarsi. */
  onReady?: () => void;
  /** Rotella, clic, tasti dentro l'email: la conversazione smette di seguire l'ultimo messaggio. */
  onUserInput?: () => void;
}) {
  const ref = useRef<HTMLIFrameElement>(null);
  const emptyRef = useRef(onEmpty);
  emptyRef.current = onEmpty;
  const widthRef = useRef(fitWidth);
  widthRef.current = fitWidth;
  const readyRef = useRef(onReady);
  readyRef.current = onReady;
  const inputRef = useRef(onUserInput);
  inputRef.current = onUserInput;
  const src = srcdoc(html, allowRemote, showQuotes, variant, showSignature);

  useLayoutEffect(() => {
    const frame = ref.current;
    if (!frame) return;
    // srcdoc si imposta qui e non come prop: così si sa quale documento è quello vecchio, che resta
    // nel riquadro finché il nuovo non arriva e non va né preparato né misurato.
    if (frame.getAttribute("srcdoc") !== src) {
      staleDocs.set(frame, frame.contentDocument);
      frame.srcdoc = src;
    }
    // Ricordato fuori dall'effetto: in sviluppo React lo esegue due volte di fila.
    const stale = staleDocs.get(frame) ?? null;
    let alive = true;
    let doc: Document | null = null; // il documento preparato e misurato
    let emptied: Document | null = null; // vuoto senza citazioni: si ricarica con le citazioni, non si misura
    let poll = 0;
    let queued = 0;
    const imgs: HTMLImageElement[] = [];

    const fit = () => {
      if (!alive || !doc || frame.contentDocument !== doc || !doc.body) return;
      const b = doc.body;
      const max = widthRef.current?.();
      if (max) {
        // Prima alla larghezza massima, per vedere dove vanno a capo le righe; poi stretto su di esse.
        frame.style.width = `${Math.floor(max)}px`;
        frame.style.width = `${Math.min(Math.floor(max), Math.max(40, contentWidth(doc) + 2))}px`;
      }
      // L'altezza del contenuto, non quella del riquadro (che all'inizio è zero).
      const r = b.getBoundingClientRect();
      const css = doc.defaultView?.getComputedStyle(b);
      const h = Math.max(r.bottom, r.top + b.scrollHeight) + (parseFloat(css?.marginBottom ?? "0") || 0);
      frame.style.height = `${Math.ceil(h)}px`;
      frame.dataset.fit = growingImages(doc) ? "img" : "ok";
      readyRef.current?.();
    };
    const refit = () => {
      if (!queued) queued = requestAnimationFrame(() => {
        queued = 0;
        fit();
      });
    };
    // I link si aprono nel browser (o nell'app di posta per mailto), mai dentro il riquadro.
    const onClick = (e: MouseEvent) => {
      const a = (e.target as Element | null)?.closest?.("a[href]");
      if (!a) return;
      e.preventDefault();
      const href = a.getAttribute("href") ?? "";
      if (/^(https?:|mailto:|tel:)/i.test(href)) openUrl(href).catch(() => {});
    };
    const onInput = () => inputRef.current?.();
    const INPUTS = ["wheel", "mousedown", "keydown", "touchstart"] as const;
    const forget = () => {
      doc?.removeEventListener("click", onClick);
      INPUTS.forEach((t) => doc?.removeEventListener(t, onInput));
      imgs.splice(0).forEach((img) => {
        img.removeEventListener("load", refit);
        img.removeEventListener("error", refit);
      });
    };
    const prepare = (d: Document) => {
      forget();
      doc = null;
      if (!showQuotes && d.body && !d.querySelector(QUOTES)) hideHeaderQuote(d);
      // Niente di visibile senza citazioni e firma (un inoltro): si ricarica con le citazioni, senza
      // misurare prima il riquadro vuoto.
      // Contano solo le immagini visibili: quelle nella citazione o nella firma nascoste no.
      const pictures = Array.from(d.images).some(
        (i) => i.getClientRects().length > 0 && i.getAttribute("width") !== "1" && i.getAttribute("height") !== "1",
      );
      if (!showQuotes && emptyRef.current && d.body && !d.body.innerText.trim() && !pictures) {
        emptied = d;
        emptyRef.current();
        return;
      }
      if (variant === "chat" && d.body) trimTail(d);
      doc = d;
      // Un link con target="_blank" in un riquadro isolato non fa niente: si apre nel riquadro, e lì
      // l'app lo ferma e lo apre nel browser (src-tauri, external_links).
      d.querySelectorAll("a[target]").forEach((a) => a.removeAttribute("target"));
      d.addEventListener("click", onClick);
      INPUTS.forEach((t) => d.addEventListener(t, onInput, { passive: true }));
      // Le immagini che arrivano dopo cambiano l'altezza: si rimisura quando arrivano.
      for (const img of Array.from(d.images)) {
        if (img.complete) continue;
        img.addEventListener("load", refit);
        img.addEventListener("error", refit);
        imgs.push(img);
      }
      fit();
    };
    /** Il documento nuovo, già letto: non quello vuoto iniziale né quello vecchio. */
    const parsed = () => {
      const d = frame.contentDocument;
      return d && d !== stale && d.URL !== "about:blank" && d.readyState !== "loading" ? d : null;
    };
    // Si controlla a ogni fotogramma, prima del disegno: `load` arriva solo dopo tutte le immagini.
    // Con i fotogrammi fermi (finestra nascosta) controlla un timer.
    const watch = () => {
      const d = parsed();
      if (d) {
        clearInterval(backup);
        prepare(d);
      } else poll = requestAnimationFrame(watch);
    };
    const backup = window.setInterval(() => {
      if (doc || emptied) return clearInterval(backup);
      const d = parsed();
      if (!d) return;
      cancelAnimationFrame(poll);
      clearInterval(backup);
      prepare(d);
    }, 40);
    watch();
    const onLoad = () => {
      const d = parsed();
      if (d && d !== doc && d !== emptied) {
        // Primo documento (fotogrammi rallentati) o ricaricato senza cambiare srcdoc (riquadro spostato).
        cancelAnimationFrame(poll);
        clearInterval(backup);
        prepare(d);
      } else {
        fit(); // tutte le immagini sono arrivate
      }
    };
    frame.addEventListener("load", onLoad);
    window.addEventListener("resize", fit);
    return () => {
      alive = false;
      cancelAnimationFrame(poll);
      clearInterval(backup);
      cancelAnimationFrame(queued);
      window.removeEventListener("resize", fit);
      frame.removeEventListener("load", onLoad);
      forget();
    };
  }, [src, showQuotes, variant]);

  // allow-same-origin senza allow-scripts: serve solo al genitore, per misurare l'altezza e
  // intercettare i clic sui link.
  return <iframe ref={ref} className={variant === "chat" ? "email-frame chat" : "email-frame"} sandbox="allow-same-origin" title="Contenuto del messaggio" />;
}
