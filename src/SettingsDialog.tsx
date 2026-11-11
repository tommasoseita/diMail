// Impostazioni (9 novembre, sera): l'AI (provider, chiave, modello), i miei domini e il client Google.
// Ogni modifica si salva subito; la finestra ha solo «Chiudi».
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, PROVIDER_NAME, type AiStatus, type GoogleClientInfo, type ModelInfo, type MyDomains, type Provider } from "./api";
import { CLIENT_GUIDE } from "./App";
import { Sparkle } from "./icons";

export type SettingsTab = "ai" | "domains" | "google";

const KEY_PAGES: Record<Provider, string> = {
  mistral: "https://console.mistral.ai/api-keys",
  openai: "https://platform.openai.com/api-keys",
};

const OTHER = "__altro__";

export function SettingsDialog({ tab: initialTab, onAiChanged, onClientChanged, onClose }: {
  tab: SettingsTab;
  onAiChanged: (status: AiStatus) => void;
  /** Il client è cambiato e serve un nuovo accesso. */
  onClientChanged: () => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  return (
    <>
      <div className="overlay" onClick={onClose} />
      <div className="dialog settings" role="dialog" aria-label="Impostazioni"
        onKeyDown={(e) => {
          if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); }
        }}>
        <h3>Impostazioni</h3>
        <div className="choices settings-tabs" role="tablist">
          <button role="tab" className={tab === "ai" ? "on" : ""} onClick={() => setTab("ai")}>AI</button>
          <button role="tab" className={tab === "domains" ? "on" : ""} onClick={() => setTab("domains")}>I miei domini</button>
          <button role="tab" className={tab === "google" ? "on" : ""} onClick={() => setTab("google")}>Account Google</button>
        </div>
        {tab === "ai" && <AiSettings onChanged={onAiChanged} />}
        {tab === "domains" && <DomainSettings />}
        {tab === "google" && <GoogleSettings onChanged={onClientChanged} />}
        <div className="buttons">
          <button className="ghost" onClick={onClose}>Chiudi</button>
        </div>
      </div>
    </>
  );
}

function AiSettings({ onChanged }: { onChanged: (status: AiStatus) => void }) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [key, setKey] = useState("");
  const [custom, setCustom] = useState("");
  const [busy, setBusy] = useState<null | "key" | "check">(null);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState<string | null>(null);
  const keyRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.aiStatus().then(setStatus).catch((e) => setError(String(e)));
    api.aiModels().then(setModels).catch(() => {});
  }, []);

  const provider = status?.provider ?? "mistral";
  const name = PROVIDER_NAME[provider];
  const hasKey = !!status?.keys.includes(provider);
  const listed = useMemo(() => models.filter((m) => m.provider === provider), [models, provider]);
  const isListed = !!status && listed.some((m) => m.id === status.model);
  const [otherOpen, setOtherOpen] = useState(false);
  const selectValue = otherOpen || (status && !isListed && listed.length > 0) ? OTHER : status?.model ?? "";

  const update = async (next: AiStatus) => {
    setStatus(next);
    onChanged(next);
  };

  const configure = async (p: Provider, model: string) => {
    setError(null);
    setChecked(null);
    try {
      await api.aiConfigure(p, model);
      await update(await api.aiStatus());
    } catch (e) {
      setError(String(e));
    }
  };

  const pickProvider = (p: Provider) => {
    if (p === provider) return;
    setKey("");
    setOtherOpen(false);
    const rec = models.find((m) => m.provider === p && m.recommended) ?? models.find((m) => m.provider === p);
    configure(p, rec?.id ?? (p === "openai" ? "gpt-5.4-mini" : "mistral-small-2603"));
  };

  const pickModel = (value: string) => {
    if (value === OTHER) {
      setOtherOpen(true);
      setCustom(isListed ? "" : status?.model ?? "");
      return;
    }
    setOtherOpen(false);
    configure(provider, value);
  };

  const saveKey = async () => {
    if (!key.trim()) return;
    setBusy("key");
    setError(null);
    try {
      await api.aiSetKey(provider, key.trim());
      setKey("");
      await update(await api.aiStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const removeKey = async () => {
    setError(null);
    try {
      await api.aiClearKey(provider);
      await update(await api.aiStatus());
    } catch (e) {
      setError(String(e));
    }
  };

  const check = async () => {
    setBusy("check");
    setError(null);
    setChecked(null);
    try {
      const ms = await api.aiCheck();
      setChecked(`Funziona: ha risposto in ${(ms / 1000).toLocaleString("it-IT", { maximumFractionDigits: 1 })} s`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  if (!status) return <p className="muted">{error ?? "…"}</p>;

  return (
    <div className="settings-pane">
      <p className="with-icon"><Sparkle size={16} style={{ color: "var(--ai)" }} />
        L'AI scrive le risposte (⌘J nel composer) e smista le email nelle sezioni con istruzioni.</p>
      <div className="form-row">
        <span>Provider</span>
        <div className="choices inline">
          {(["mistral", "openai"] as Provider[]).map((p) => (
            <button key={p} className={p === provider ? "on" : ""} onClick={() => pickProvider(p)}>
              {PROVIDER_NAME[p]}{status.keys.includes(p) ? " · chiave ✓" : ""}
            </button>
          ))}
        </div>
      </div>
      <label className="form-row">
        <span>Chiave API di {name}</span>
        <div className="input-with-button">
          <input ref={keyRef} type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" spellCheck={false}
            onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); saveKey(); } }}
            placeholder={hasKey ? "Già impostata: incollane una nuova per cambiarla" : "Incolla la chiave"} />
          <button className="primary" onClick={saveKey} disabled={busy !== null || !key.trim()}>{busy === "key" ? "Provo…" : "Salva"}</button>
        </div>
        <span className="hint">
          La chiave resta nel Portachiavi del Mac. <button className="link-btn" onClick={() => openUrl(KEY_PAGES[provider]).catch(() => {})}>Crea una chiave su {name}</button>
          {hasKey && <> · <button className="link-btn" onClick={removeKey}>Togli la chiave</button></>}
        </span>
      </label>
      <label className="form-row">
        <span>Modello</span>
        <select value={selectValue} onChange={(e) => pickModel(e.target.value)}>
          {listed.map((m) => <option key={m.id} value={m.id}>{m.name}{m.recommended ? " (consigliato)" : ""}</option>)}
          <option value={OTHER}>Altro…</option>
        </select>
        {selectValue === OTHER && (
          <div className="input-with-button">
            <input value={custom} onChange={(e) => setCustom(e.target.value)} placeholder="Nome del modello, es. mistral-small-latest" spellCheck={false}
              onKeyDown={(e) => { if (e.key === "Enter" && custom.trim()) { e.preventDefault(); configure(provider, custom.trim()); } }} />
            <button className="ghost" onClick={() => configure(provider, custom.trim())} disabled={!custom.trim() || custom.trim() === status.model}>Usa</button>
          </div>
        )}
        <span className="hint">In uso: {status.model}. Il testo delle email coinvolte va a {name}.</span>
      </label>
      <div className="check-line">
        <button className="ghost" onClick={check} disabled={busy !== null || !hasKey}>{busy === "check" ? "Provo…" : "Prova"}</button>
        {checked && <span className="ok small">{checked}</span>}
        <span className="grow" />
        <span className="muted small">Oggi {status.tokens_today.toLocaleString("it-IT")} token su {status.daily_cap.toLocaleString("it-IT")}</span>
      </div>
      {error && <p className="error small">{error}</p>}
    </div>
  );
}

function DomainSettings() {
  const [view, setView] = useState<MyDomains | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.myDomains().then(setView).catch((e) => setError(String(e)));
  }, []);

  const save = async (domains: string[]) => {
    setBusy(true);
    setError(null);
    try {
      setView(await api.setMyDomains(domains));
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const add = async () => {
    const d = draft.trim();
    if (!d || !view) return;
    if (await save([...view.domains, d])) setDraft("");
  };

  if (!view) return <p className="muted">{error ?? "…"}</p>;

  return (
    <div className="settings-pane">
      <p>I messaggi che arrivano da questi domini vanno sempre in Importanti e le loro immagini si caricano subito.
        Di solito è il dominio della tua azienda.</p>
      <div className="domain-chips">
        {view.domains.map((d) => (
          <span key={d} className="domain-chip">@{d}
            <button aria-label={`Togli ${d}`} title="Togli" disabled={busy} onClick={() => save(view.domains.filter((x) => x !== d))}>×</button>
          </span>
        ))}
        {view.domains.length === 0 && <span className="muted small">Nessun dominio.</span>}
      </div>
      <div className="form-row">
        <div className="input-with-button">
          <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="azienda.it" spellCheck={false} autoCapitalize="off"
            onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); add(); } }} />
          <button className="ghost" onClick={add} disabled={busy || !draft.trim()}>Aggiungi</button>
        </div>
        <span className="hint">
          {!view.custom && view.suggested && `Ricavato dal tuo indirizzo.`}
          {!view.custom && !view.suggested && "Con un indirizzo Gmail non c'è un dominio aziendale: aggiungine uno se ti serve."}
          {view.custom && view.suggested && !view.domains.includes(view.suggested) && (
            <>Il dominio del tuo indirizzo non c'è. <button className="link-btn" onClick={() => save([...view.domains, view.suggested!])}>Aggiungi @{view.suggested}</button></>
          )}
        </span>
      </div>
      {error && <p className="error small">{error}</p>}
    </div>
  );
}

function GoogleSettings({ onChanged }: { onChanged: () => void }) {
  const [info, setInfo] = useState<GoogleClientInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.googleClient().then(setInfo).catch((e) => setError(String(e)));
  }, []);

  const [copied, setCopied] = useState(false);
  const copyLink = async () => {
    setError(null);
    try {
      const link = await api.clientSetupLink();
      try {
        await navigator.clipboard.writeText(link);
      } catch {
        // Senza l'API degli appunti: il vecchio comando di copia, su un campo temporaneo.
        const field = document.createElement("textarea");
        field.value = link;
        field.style.position = "fixed";
        field.style.opacity = "0";
        document.body.appendChild(field);
        field.select();
        const ok = document.execCommand("copy");
        field.remove();
        if (!ok) throw new Error("non riesco a copiare negli appunti");
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 2500);
    } catch (e) {
      setError(String(e));
    }
  };

  const replace = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await api.loadGoogleClient();
      if (next) {
        const changed = next.client_id !== info?.client_id;
        setInfo(next);
        if (changed) onChanged();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-pane">
      <p>diMail parla con Gmail tramite il tuo client OAuth di Google. Il refresh token e il client restano nel Portachiavi del Mac.</p>
      {info && (
        <dl className="client-info">
          <dt>Progetto</dt><dd>{info.project_id ?? "—"}</dd>
          <dt>Client</dt><dd className="mono">{info.client_id}</dd>
        </dl>
      )}
      <div className="check-line">
        <button className="ghost" onClick={replace} disabled={busy}>{busy ? "Leggo il file…" : "Sostituisci il JSON"}</button>
        {info && <button className="ghost" onClick={copyLink} title="Chi apre il link configura diMail con questo client, senza il file JSON">{copied ? "Link copiato" : "Copia il link di configurazione"}</button>}
        <span className="grow" />
        <button className="link-btn" onClick={() => openUrl(CLIENT_GUIDE).catch(() => {})}>Come si crea il client</button>
      </div>
      <p className="muted small">Con un client diverso dovrai rifare l'accesso a Google. La posta già scaricata resta.</p>
      <p className="muted small">Il link di configurazione contiene il client, come il file JSON: mandalo solo ai colleghi che devono usare diMail. Ognuno accede poi con il proprio account.</p>
      {error && <p className="error small">{error}</p>}
    </div>
  );
}
