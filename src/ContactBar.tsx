// Barra del contatto (specifica UX, sezione 4), come pannello a destra: solo dati della posta.
import { useEffect, useRef, useState } from "react";
import { api, type Addr, type ContactCard } from "./api";
import { avatarColor, count, initials, whenLabel } from "./format";
import { PanelRight } from "./icons";

const cache = new Map<string, { at: number; card: ContactCard }>();

export function ContactBar({ person, onOpenThread, onCard, onClose }: {
  person: Addr | null;
  onOpenThread: (id: string) => void;
  /** Per i tasti 1–5, che aprono le conversazioni recenti. */
  onCard?: (card: ContactCard | null) => void;
  onClose: () => void;
}) {
  // Persona e scheda cambiano insieme (11 novembre): passando col mouse da una conversazione all'altra
  // non si vedono più il nome nuovo con i dati vecchi, né la barra che si svuota e si riempie.
  const [shown, setShown] = useState<{ person: Addr | null; card: ContactCard | null }>({ person, card: null });
  const email = person?.email ?? null;
  const personRef = useRef(person);
  personRef.current = person;
  const first = useRef(true);

  useEffect(() => onCard?.(shown.card), [shown.card, onCard]);

  useEffect(() => {
    const who = personRef.current;
    if (!email || !who) return setShown({ person: null, card: null });
    let alive = true;
    // Piccola attesa: scorrendo veloce (mouse o J/K) la barra non cambia a ogni riga attraversata.
    const t = setTimeout(() => {
      const hit = cache.get(email);
      if (hit && Date.now() - hit.at < 60_000) return setShown({ person: who, card: hit.card });
      api
        .contactCard(email)
        .then((c) => {
          cache.set(email, { at: Date.now(), card: c });
          if (alive) setShown({ person: who, card: c });
        })
        .catch(() => alive && setShown({ person: who, card: null }));
    }, first.current ? 0 : 110);
    first.current = false;
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [email]);

  const { card } = shown;
  const shownPerson = shown.person;
  const name = card?.name || shownPerson?.name || null;
  const since = card?.first_date ? new Date(card.first_date).toLocaleDateString("it-IT", { month: "long", year: "numeric" }) : null;

  return (
    <aside className="panel" aria-label="Contatto">
      <div className="panel-head">
        <button className="icon-btn" onClick={onClose} title="Chiudi (I)">
          <PanelRight />
        </button>
        <span className="title">Contatto</span>
      </div>
      <div className="panel-body contact" key={shownPerson?.email ?? ""}>
        {!shownPerson && <p className="muted">Nessun contatto per questo thread.</p>}
        {shownPerson && (
          <>
            <div className="avatar" style={{ background: avatarColor(shownPerson.email) }}>{initials(name, shownPerson.email)}</div>
            <h3>{name || shownPerson.email}</h3>
            <div className="email">{shownPerson.email}</div>
            {card?.role && <div className="fact">{card.role}</div>}
            {card?.company && <div className="fact">{card.company}</div>}
            {card?.phone && <div className="fact num">{card.phone}</div>}

            {card && (card.received_count > 0 || card.sent_count > 0) && (
              <div className="section">
                <h4>Posta</h4>
                <div className="fact">
                  {count(card.received_count)} ricevuti · {count(card.sent_count)} inviati
                </div>
                {since && <div className="fact muted small">dal {since}</div>}
              </div>
            )}

            {card && card.recent.length > 0 && (
              <div className="section recent">
                <h4>Conversazioni recenti</h4>
                {card.recent.map((t, i) => (
                  <button key={t.id} onClick={() => onOpenThread(t.id)} title={t.subject}>
                    <kbd>{i + 1}</kbd>
                    <span className="s">{t.subject || "(nessun oggetto)"}</span>
                    <span className="d">{whenLabel(t.last_date)}</span>
                  </button>
                ))}
              </div>
            )}
          </>
        )}
      </div>
    </aside>
  );
}
