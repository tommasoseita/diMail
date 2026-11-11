// File trascinati dal Finder sulla finestra. Tauri li intercetta (con i percorsi veri), quindi niente
// drag and drop HTML per i file: si ascoltano i suoi eventi. I trascinamenti interni (le sezioni nella
// barra) non hanno percorsi e non contano.
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef } from "react";

export function useFileDrop(active: boolean, on: { over: (dragging: boolean) => void; drop: (paths: string[]) => void }) {
  const ref = useRef(on);
  ref.current = on;
  useEffect(() => {
    if (!active) return;
    let alive = true;
    let unlisten: (() => void) | null = null;
    try {
      getCurrentWebview()
        .onDragDropEvent((e) => {
          const p = e.payload;
          if (p.type === "enter") ref.current.over(p.paths.length > 0);
          else if (p.type === "leave") ref.current.over(false);
          else if (p.type === "drop") {
            ref.current.over(false);
            if (p.paths.length) ref.current.drop(p.paths);
          }
        })
        .then((f) => {
          if (alive) unlisten = f;
          else f();
        })
        .catch(() => {});
    } catch {
      // Fuori da Tauri (anteprima nel browser): niente file trascinati.
    }
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [active]);
}
