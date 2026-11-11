// ⌘+ e ⌘− ingrandiscono o rimpiccioliscono tutta l'interfaccia, testo delle email compreso, e ⌘0 la
// riporta alla dimensione normale (11 novembre). Come in Safari: è lo zoom della pagina della webview
// (`setZoom` di Tauri), e diMail se lo ricorda tra un avvio e l'altro.
import { getCurrentWebview } from "@tauri-apps/api/webview";

const STEPS = [0.8, 0.9, 1, 1.1, 1.2, 1.3, 1.45, 1.6];
const KEY = "dimail.zoom";

let level = 1;

function stored(): number {
  try {
    const v = Number(localStorage.getItem(KEY));
    return STEPS.includes(v) ? v : 1;
  } catch {
    return 1;
  }
}

async function apply(z: number) {
  level = z;
  document.documentElement.style.setProperty("--zoom", String(z));
  try {
    localStorage.setItem(KEY, String(z));
  } catch {
    // senza memoria del browser vale fino alla chiusura
  }
  try {
    await getCurrentWebview().setZoom(z);
  } catch {
    // Nel browser (sviluppo, backend finto) non c'è la webview di Tauri.
    document.documentElement.style.setProperty("zoom", z === 1 ? "" : String(z));
  }
}

export function installZoom() {
  level = stored();
  if (level !== 1) apply(level);
  window.addEventListener(
    "keydown",
    (e) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      const i = STEPS.indexOf(level);
      let next: number;
      // Sulla tastiera italiana "+" ha il suo tasto; su quella americana è Maiuscole+"=".
      if (e.key === "+" || e.key === "=") next = STEPS[Math.min(STEPS.length - 1, i + 1)];
      else if (e.key === "-" || e.key === "_") next = STEPS[Math.max(0, i - 1)];
      else if (e.key === "0") next = 1;
      else return;
      e.preventDefault();
      e.stopPropagation();
      if (next !== level) apply(next);
    },
    true,
  );
}
