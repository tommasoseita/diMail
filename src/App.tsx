import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useState } from "react";
import { api, onAppReady, onCoreEvent, type AppInfo, type SetupLinkInfo } from "./api";
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

  // Link di configurazione (dimail://client?c=…): aperto da fuori, anche ad app chiusa, o incollato
  // nella prima schermata. Si chiede sempre conferma prima di usarlo.
  const [link, setLink] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    let stop: (() => void) | undefined;
    const take = (urls: string[] | null) => {
      const url = urls?.find((u) => u.startsWith(SETUP_LINK));
      if (url && alive) setLink(url);
    };
    getCurrent().then(take).catch(() => {});
    onOpenUrl(take).then((f) => (alive ? (stop = f) : f())).catch(() => {});
    return () => {
      alive = false;
      stop?.();
    };
  }, []);

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
  const screen = info.needs_client ? <ClientSetup hint={info.login_hint} onDone={refresh} onLink={setLink} />
    : !info.account || (info.needs_login && !info.account) ? <Login hint={info.login_hint} onDone={refresh} />
    : <Workspace account={info.account} needsLogin={info.needs_login} onRelogin={refresh} />;
  return (
    <>
      {screen}
      {link && <SetupLinkDialog link={link} onClose={() => setLink(null)} onDone={refresh} />}
    </>
  );
}

const SETUP_LINK = "dimail://client";

/** Conferma di un link di configurazione: dice quale client imposta e cosa cambia. */
function SetupLinkDialog({ link, onClose, onDone }: { link: string; onClose: () => void; onDone: () => void }) {
  const [info, setInfo] = useState<SetupLinkInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    setInfo(null);
    setError(null);
    api.setupLinkInfo(link).then((i) => alive && setInfo(i), (e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [link]);

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", k, true);
    return () => window.removeEventListener("keydown", k, true);
  }, [onClose]);

  const apply = async () => {
    setBusy(true);
    try {
      await api.applySetupLink(link);
      onDone();
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <>
      <div className="overlay" onClick={onClose} />
      <div className="dialog" role="dialog" aria-label="Link di configurazione">
        <h3>{info?.same ? "diMail è già configurata" : "Configurare diMail con questo link?"}</h3>
        {!info && !error && <p>Leggo il link…</p>}
        {info && (
          <>
            <p>
              {info.same
                ? "Questo link porta il client Google che diMail usa già: non c'è niente da cambiare."
                : "Il link imposta il client Google con cui diMail parla con Gmail. Usalo solo se te l'ha mandato qualcuno di cui ti fidi, per esempio la tua azienda."}
            </p>
            <dl className="client-info">
              <dt>Progetto</dt><dd>{info.project_id ?? "—"}</dd>
              <dt>Client</dt><dd className="mono">{info.client_id}</dd>
            </dl>
            {info.replaces && <p className="small">Sostituisce il client di adesso: dovrai rifare l'accesso a Google. La posta già scaricata resta.</p>}
          </>
        )}
        {error && <p className="error small">{error}</p>}
        <div className="buttons">
          {info && !info.same ? (
            <>
              <button className="ghost" onClick={onClose} disabled={busy}>Annulla</button>
              <button className="primary" onClick={apply} disabled={busy}>{busy ? "Configuro…" : "Configura"}</button>
            </>
          ) : (
            <button className="primary" onClick={onClose}>Chiudi</button>
          )}
        </div>
      </div>
    </>
  );
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
function ClientSetup({ hint, onDone, onLink }: { hint: string | null; onDone: () => void; onLink: (link: string) => void }) {
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

  // Il link di configurazione si può anche incollare qui (⌘V), se aprirlo non ha avviato diMail.
  useEffect(() => {
    const paste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData("text/plain").trim() ?? "";
      if (!text.startsWith(SETUP_LINK)) return;
      e.preventDefault();
      onLink(text);
    };
    window.addEventListener("paste", paste);
    return () => window.removeEventListener("paste", paste);
  }, [onLink]);

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
        <p className="small muted">La tua azienda ti ha mandato un link di configurazione? Aprilo, oppure incollalo qui con ⌘V.</p>
      </div>
    </div>
  );
}
