import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useState } from "react";
import { api, onAppReady, onCoreEvent, type AppInfo } from "./api";
import { describe, report } from "./diagnostics";
import { useFileDrop } from "./fileDrop";
import { Workspace } from "./Workspace";
import logo from "./logo.png";
import "./App.css";

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .appInfo()
      .then(setInfo)
      .catch((e) => {
        report("error", `app_info: ${describe(e)}`);
        setError(String(e));
      });
  }, []);

  useEffect(() => {
    const ready = onAppReady(refresh);
    const un = onCoreEvent((e) => {
      if (e.type === "auth_required") refresh();
    });
    // Dopo aver messo in ascolto `app_ready`, così l'evento non si perde.
    ready.then(refresh);
    return () => {
      ready.then((f) => f());
      un.then((f) => f());
    };
  }, [refresh]);

  if (error) return <div className="center error">{error}</div>;
  if (!info || info.starting) {
    return (
      <div className="center brand-bg">
        <div className="login">
          <img className="logo" src={logo} alt="" />
          <p>Avvio…</p>
          {info?.starting && <p className="small">Se macOS chiede l'accesso al Portachiavi per diMail, scegli «Consenti sempre».</p>}
        </div>
      </div>
    );
  }
  if (info.needs_client) return <ClientSetup hint={info.login_hint} onDone={refresh} />;
  if (!info.account || (info.needs_login && !info.account)) {
    return <Login hint={info.login_hint} onDone={refresh} />;
  }
  return <Workspace account={info.account} needsLogin={info.needs_login} onRelogin={refresh} />;
}

export function Login({ hint, onDone, compact }: { hint: string | null; onDone: () => void; compact?: boolean }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.login();
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const button = (
    <button className="primary" onClick={start} disabled={busy}>
      {busy ? "Completa l'accesso nel browser…" : hint ? "Rifai il login" : "Accedi con Google"}
    </button>
  );

  if (compact) {
    return (
      <span className="relogin">
        {button}
        {error && <span className="error"> {error}</span>}
      </span>
    );
  }

  return (
    <div className="center brand-bg">
      <div className="login">
        <img className="logo" src={logo} alt="" />
        <h1>diMail</h1>
        <p className="muted">
          {hint ? `L'accesso di ${hint} è scaduto. I dati locali sono al sicuro.` : "Collega il tuo account Gmail o Google Workspace."}
        </p>
        {button}
        {busy && <p className="muted small">Si è aperto il browser. Torna qui quando hai finito.</p>}
        {error && <p className="error small">{error}</p>}
      </div>
    </div>
  );
}

/** La guida per creare il client Google (docs/it/google-client.md). */
export const CLIENT_GUIDE = "https://github.com/tommasoseita/diMail/blob/main/docs/it/google-client.md";

/**
 * Primo avvio (modalità personale): diMail parla con Gmail tramite un client OAuth dell'utente, che
 * ne carica il JSON scaricato dalla console di Google Cloud. Si sceglie dal pannello di macOS o si
 * trascina sulla finestra.
 */
function ClientSetup({ hint, onDone }: { hint: string | null; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = async (path?: string) => {
    setBusy(true);
    setError(null);
    try {
      if (await api.loadGoogleClient(path)) onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  useFileDrop(true, { over: setDragging, drop: (paths) => load(paths[0]) });

  return (
    <div className="center brand-bg">
      <div className={`login setup${dragging ? " dragging" : ""}`}>
        <img className="logo" src={logo} alt="" />
        <h1>{hint ? "diMail è cambiata" : "Benvenuto in diMail"}</h1>
        {hint ? (
          <p className="muted">
            La posta di {hint} è qui, intatta. Adesso diMail usa un client Google tuo:
            carica lo stesso JSON di prima e riparte senza rifare l'accesso.
          </p>
        ) : (
          <p className="muted">
            diMail parla con Gmail direttamente dal tuo Mac. Per farlo usa un client OAuth di Google tuo:
            lo crei una volta nella console di Google Cloud e carichi qui il file JSON.
          </p>
        )}
        <div className="drop-zone">
          <span>{dragging ? "Rilascia il file" : "Trascina qui il file JSON del client"}</span>
        </div>
        <button className="primary" onClick={() => load()} disabled={busy}>
          {busy ? "Leggo il file…" : "Scegli il file JSON"}
        </button>
        {error && <p className="error small">{error}</p>}
        <p className="small muted">
          Non l'hai ancora?{" "}
          <button className="link-btn" onClick={() => openUrl(CLIENT_GUIDE).catch(() => {})}>Come si crea il client</button>
          {" "}(una decina di minuti, una volta sola).
        </p>
      </div>
    </div>
  );
}
