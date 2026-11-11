// Errori dell'interfaccia nel log dell'app (~/Library/Logs/diMail): nella build di release la console
// della WebView non si vede. Si importa per primo in main.tsx. Mai contenuto delle email nei messaggi.
import { invoke } from "@tauri-apps/api/core";

type Level = "error" | "warn" | "info";

export function report(level: Level, message: string) {
  invoke("log_frontend", { level, message: message.slice(0, 2000) }).catch(() => {});
}

function describe(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}\n${e.stack ?? ""}`;
  return String(e);
}

window.addEventListener("error", (e) => {
  report("error", `${e.message} (${e.filename}:${e.lineno}:${e.colno})\n${e.error instanceof Error ? (e.error.stack ?? "") : ""}`);
});

window.addEventListener("unhandledrejection", (e) => {
  report("error", `promessa rifiutata: ${describe(e.reason)}`);
});

document.addEventListener("securitypolicyviolation", (e) => {
  report("error", `bloccato dalla CSP: ${e.violatedDirective} ${e.blockedURI} (${e.sourceFile}:${e.lineNumber})`);
});

export { describe };
