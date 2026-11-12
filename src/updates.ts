// Aggiornamenti automatici (11 novembre): diMail controlla l'ultima release pubblicata su GitHub
// all'avvio e poi ogni sei ore, la scarica in background e ne verifica la firma (plugin updater di
// Tauri, chiave pubblica in tauri.conf.json). Poi lo dice in basso a sinistra: «diMail 0.3.0 è
// pronta · Riavvia». Non si riavvia mai da sola: le bozze nel composer vivono solo in memoria.
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { useEffect, useState } from "react";
import { describe, report } from "./diagnostics";

const FIRST_CHECK = 15_000;
const EVERY = 6 * 60 * 60 * 1000;

/** La versione già scaricata e installata, pronta al prossimo avvio; `null` finché non c'è. */
export function useUpdate(): { version: string | null; restart: () => Promise<void> } {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    // In sviluppo (tauri dev o il backend finto nel browser) non si aggiorna niente.
    if (import.meta.env.DEV) return;
    let alive = true;
    let busy = false;
    let done = false;
    const look = async () => {
      if (busy || done) return;
      busy = true;
      try {
        const update = await check();
        if (update) {
          await update.downloadAndInstall();
          done = true;
          report("info", `aggiornamento ${update.version} installato, attivo dal prossimo avvio`);
          if (alive) setVersion(update.version);
        }
      } catch (e) {
        // Senza rete, o release non ancora pubblicata: si riprova al prossimo giro.
        report("warn", `aggiornamento: ${describe(e).split("\n")[0]}`);
      } finally {
        busy = false;
      }
    };
    const first = setTimeout(look, FIRST_CHECK);
    const timer = setInterval(look, EVERY);
    return () => {
      alive = false;
      clearTimeout(first);
      clearInterval(timer);
    };
  }, []);

  // Non il riavvio di Tauri, che apre una seconda istanza e raddoppia l'icona nel Dock: diMail si
  // chiude e si riapre (src-tauri, restart_app).
  return { version, restart: () => invoke<void>("restart_app") };
}
