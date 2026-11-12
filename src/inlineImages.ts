// Le immagini incollate nel corpo di un'email (12 novembre): nell'HTML sono `<img src="cid:…">` e il
// file è un allegato del messaggio con quel Content-ID. diMail le scarica quando la conversazione si
// apre e le mette al loro posto. Non sono immagini remote: arrivano con l'email, niente tracciamento,
// quindi si caricano anche per i mittenti sconosciuti.
import { api, type AttachmentView, type MessageView } from "./api";
import { describe, report } from "./diagnostics";

/** Già scaricate, per riaprire la conversazione senza aspettare. Le più vecchie escono oltre il tetto. */
const ready = new Map<string, string>();
const loading = new Map<string, Promise<string | null>>();
const MAX_CHARS = 48_000_000;
let chars = 0;

function keep(key: string, url: string) {
  ready.set(key, url);
  chars += url.length;
  for (const [k, v] of ready) {
    if (chars <= MAX_CHARS || k === key) break;
    ready.delete(k);
    chars -= v.length;
  }
}

function load(messageId: string, partId: string): string | Promise<string | null> {
  const key = `${messageId}/${partId}`;
  const have = ready.get(key);
  if (have) return have;
  let p = loading.get(key);
  if (!p) {
    p = api.inlineImage(messageId, partId).then(
      (url) => {
        keep(key, url);
        return url;
      },
      (e) => {
        // Resta il nome del file al posto dell'immagine. Nel registro la causa, mai il contenuto.
        report("warn", `immagine nel testo non caricata: ${describe(e).split("\n")[0]}`);
        return null;
      },
    ).finally(() => loading.delete(key));
    loading.set(key, p);
  }
  return p;
}

/** `cid:image001.png@01DC…` → `image001.png@01DC…` */
function cidOf(src: string): string {
  let id = src.replace(/^cid:/i, "").trim();
  try {
    id = decodeURIComponent(id);
  } catch {
    // resta com'è
  }
  return id.replace(/^<|>$/g, "");
}

function partFor(attachments: AttachmentView[], src: string): AttachmentView | undefined {
  const id = cidOf(src);
  if (!id) return undefined;
  return attachments.find((a) => a.content_id === id) ?? attachments.find((a) => a.content_id?.toLowerCase() === id.toLowerCase());
}

/** Per un messaggio: da `cid:…` all'immagine (subito se già scaricata), o `null` se non è sua. */
export function inlineImagesOf(m: Pick<MessageView, "id" | "attachments">): (src: string) => string | Promise<string | null> | null {
  return (src) => {
    const part = partFor(m.attachments, src);
    return part ? load(m.id, part.part_id) : null;
  };
}

/** L'allegato è un'immagine mostrata nel testo: non serve anche come scheda tra gli allegati. */
export function shownInline(a: AttachmentView, html: string | null): boolean {
  if (a.is_inline) return true;
  return !!a.content_id && !!html && html.toLowerCase().includes(`cid:${a.content_id.toLowerCase()}`);
}
