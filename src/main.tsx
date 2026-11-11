import "./diagnostics"; // per primo: raccoglie gli errori anche durante l'avvio
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { report } from "./diagnostics";
import { ErrorBoundary } from "./ErrorBoundary";
import { installZoom } from "./zoom";

async function start() {
  // Nel browser, in sviluppo, un backend finto con dati inventati (src/dev/mockTauri.ts).
  // Nella build questo ramo non c'è: import.meta.env.DEV è falso e il modulo non viene incluso.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    await import("./dev/mockTauri");
  }
  report("info", "interfaccia avviata");
  installZoom();
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>,
  );
}

start();
