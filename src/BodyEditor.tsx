// Il testo dell'email nel composer (11 novembre): un campo modificabile che mostra i link.
//
// Il modello resta testo semplice, come nel textarea di prima: una riga per riga, e i link scritti
// `[testo](url)`. Il nucleo li trasforma all'invio (<a href> nell'HTML, "testo <url>" nel testo
// semplice), e bozza, coda d'invio, ⌘J e "Riapri" non cambiano.
//
// Selezionare del testo e incollare un URL (⌘V) ne fa un link, con il comando nativo di WebKit:
// ⌘Z lo annulla. Ogni altro incolla entra come testo semplice, senza la formattazione di chi l'ha
// copiato.
import { forwardRef, useImperativeHandle, useLayoutEffect, useRef } from "react";

export type BodyEditorHandle = {
  focus(): void;
  /** Cursore in fondo al testo. */
  focusEnd(): void;
};

const URL_ONLY = /^(https?:\/\/|mailto:)\S+$/i;
const LINK = /\[([^[\]\n]+)\]\(((?:https?:\/\/|mailto:)[^\s)]+)\)/g;
const BLOCKS = new Set(["DIV", "P", "LI", "UL", "OL", "BLOCKQUOTE", "PRE", "H1", "H2", "H3", "H4", "H5", "H6", "TABLE", "TR"]);

function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Un URL da incollare come link: http(s), mailto, o "www.…" (diventa https://). */
export function asUrl(text: string): string | null {
  const t = text.trim();
  if (URL_ONLY.test(t)) return t;
  if (/^www\.[^\s.]+\.\S+$/i.test(t)) return `https://${t}`;
  return null;
}

/** Dal testo della bozza al contenuto del campo: una riga per `<div>`, i link come `<a>`. */
export function toHtml(value: string): string {
  return value
    .split("\n")
    .map((line) => {
      if (!line) return "<div><br></div>";
      let html = "";
      let last = 0;
      for (const m of line.matchAll(LINK)) {
        if (!m[1].trim()) continue;
        html += esc(line.slice(last, m.index)) + `<a href="${esc(m[2])}" title="${esc(m[2])}">${esc(m[1])}</a>`;
        last = m.index! + m[0].length;
      }
      return `<div>${html + esc(line.slice(last))}</div>`;
    })
    .join("");
}

/**
 * Dal campo al testo della bozza. WebKit scrive la prima riga come testo e le altre come `<div>`,
 * le righe vuote come `<div><br></div>`, gli a capo con Maiuscole+Invio come `<br>`.
 */
export function fromDom(root: Node): string {
  const lines: string[] = [];
  let line = "";
  let open = false;
  const end = () => {
    lines.push(line);
    line = "";
    open = false;
  };
  const visit = (node: Node, inBlock: boolean) => {
    const children = Array.from(node.childNodes);
    children.forEach((child, i) => {
      if (child.nodeType === Node.TEXT_NODE) {
        line += (child.textContent ?? "").replace(/\u00a0/g, " ");
        open = true;
        return;
      }
      if (child.nodeType !== Node.ELEMENT_NODE) return;
      const el = child as HTMLElement;
      if (el.tagName === "BR") {
        // Il <br> finale di un blocco con del testo è solo un segnaposto di WebKit.
        if (i === children.length - 1 && inBlock && open) return;
        end();
        return;
      }
      if (el.tagName === "A") {
        const href = el.getAttribute("href") ?? "";
        const text = (el.textContent ?? "").replace(/\u00a0/g, " ");
        if (URL_ONLY.test(href) && text.trim()) {
          const safeText = text.replace(/\[/g, "(").replace(/\]/g, ")");
          line += `[${safeText}](${href.replace(/\(/g, "%28").replace(/\)/g, "%29")})`;
        } else {
          line += text;
        }
        open = true;
        return;
      }
      if (BLOCKS.has(el.tagName)) {
        if (open) end();
        visit(el, true);
        if (open) end();
        return;
      }
      visit(el, false);
    });
  };
  visit(root, true);
  if (open) end();
  return lines.join("\n");
}

type Props = {
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  placeholder?: string;
  className?: string;
  /** Oltre questa altezza il campo scorre (nel pannello ingrandito nessun limite). */
  maxHeight?: number;
};

export const BodyEditor = forwardRef<BodyEditorHandle, Props>(function BodyEditor({ value, onChange, readOnly, placeholder, className, maxHeight }, ref) {
  const el = useRef<HTMLDivElement>(null);
  // L'ultimo valore uscito dal campo: quando torna dal genitore non si ridisegna, e il cursore resta.
  const emitted = useRef<string | null>(null);
  const changeRef = useRef(onChange);
  changeRef.current = onChange;

  useLayoutEffect(() => {
    const div = el.current;
    if (!div || value === emitted.current) return;
    div.innerHTML = toHtml(value);
    emitted.current = value;
  }, [value]);

  const emit = () => {
    const div = el.current;
    if (!div) return;
    const next = fromDom(div);
    if (next === emitted.current) return;
    emitted.current = next;
    changeRef.current(next);
  };

  useImperativeHandle(ref, () => ({
    focus: () => el.current?.focus(),
    focusEnd: () => {
      const div = el.current;
      if (!div) return;
      div.focus();
      const range = document.createRange();
      range.selectNodeContents(div);
      range.collapse(false);
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
    },
  }), []);

  const onPaste = (e: React.ClipboardEvent<HTMLDivElement>) => {
    const text = e.clipboardData.getData("text/plain");
    e.preventDefault();
    if (!text || readOnly) return;
    const sel = window.getSelection();
    const url = asUrl(text);
    const selected = !!sel && !sel.isCollapsed && !!el.current?.contains(sel.anchorNode);
    // execCommand è deprecato ma è l'unico modo che tiene ⌘Z: WebKit lo supporta.
    if (url && selected) document.execCommand("createLink", false, url);
    else document.execCommand("insertText", false, text);
    emit();
  };

  // Grassetto e corsivo (⌘B, ⌘I, ⌘U) il campo li farebbe da sé, ma l'email parte in testo semplice.
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if ((e.metaKey || e.ctrlKey) && !e.altKey && ["b", "i", "u"].includes(e.key.toLowerCase())) e.preventDefault();
  };

  // Testo trascinato: entra come testo semplice, come quello incollato.
  const onDrop = (e: React.DragEvent<HTMLDivElement>) => {
    const text = e.dataTransfer.getData("text/plain");
    if (!text || readOnly) return;
    e.preventDefault();
    const doc = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
    const range = doc.caretRangeFromPoint?.(e.clientX, e.clientY);
    if (range) {
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
    }
    document.execCommand("insertText", false, text);
    emit();
  };

  return (
    <div ref={el} className={className} contentEditable={!readOnly} suppressContentEditableWarning role="textbox" aria-multiline
      aria-placeholder={placeholder} spellCheck data-placeholder={placeholder} data-empty={value.trim() === "" ? "" : undefined}
      style={{ maxHeight: maxHeight ?? "none" }} onInput={emit} onPaste={onPaste} onKeyDown={onKeyDown} onDrop={onDrop} />
  );
});
