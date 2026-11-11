# Creare il client Google per diMail

diMail parla con Gmail direttamente dal tuo Mac, senza server in mezzo. Per farlo Google vuole un
"client OAuth": l'identità dell'app che chiede l'accesso alla tua posta. In modalità personale il
client lo crei tu, nel tuo progetto Google Cloud, e diMail lo usa solo sul tuo Mac. Ci vogliono una
decina di minuti, una volta sola.

Se la tua azienda usa Google Workspace, basta un client solo per tutti: vedi
[Un'azienda su Google Workspace](#unazienda-su-google-workspace) in fondo.

In English: [docs/google-client.md](../google-client.md).

## Prima di cominciare: Interno o Esterno

| Il tuo indirizzo | Pubblico del client | Cosa comporta |
|---|---|---|
| Google Workspace (es. `nome@azienda.it`) | **Interno** | Possono accedere solo gli account della tua organizzazione. Nessuna verifica di Google, nessun avviso al login. Serve essere amministratore del Workspace, o chiedere a chi lo è. |
| Gmail personale (`@gmail.com`) | **Esterno**, stato **In produzione** | Al login Google avvisa che l'app non è verificata: è normale, l'app è la tua. In stato "Test" l'accesso scadrebbe ogni 7 giorni. |

## 1. Il progetto

1. Apri [console.cloud.google.com](https://console.cloud.google.com) con l'account a cui collegherai diMail.
2. In alto, dal selettore dei progetti, scegli **Nuovo progetto**. Nome: `diMail`. Crea e selezionalo.

## 2. La Gmail API

1. Menu → **API e servizi** → **Libreria**.
2. Cerca **Gmail API**, aprila e premi **Abilita**.

## 3. La schermata di consenso

Menu → **API e servizi** → **Schermata consenso OAuth** (si apre la sezione Google Auth Platform) →
**Inizia**.

1. Informazioni sull'app: nome `diMail`, email di assistenza (la tua).
2. Pubblico: **Interno** per Google Workspace, **Esterno** per Gmail personale (vedi la tabella sopra).
3. Informazioni di contatto: la tua email. Accetta le norme e premi **Crea**.
4. Facoltativo ma utile: in **Accesso ai dati** → **Aggiungi o rimuovi ambiti** aggiungi
   `https://www.googleapis.com/auth/gmail.modify`. diMail chiede solo questo, più nome, email e foto
   del profilo (`openid`, `email`, `profile`).
5. Solo per **Esterno**: in **Pubblico** premi **Pubblica app** e conferma. Lo stato passa a
   "In produzione". Non serve chiedere la verifica a Google per usarla da solo.

## 4. Il client

1. In Google Auth Platform → **Client** → **Crea client**.
2. Tipo di applicazione: **App desktop**. Nome: `diMail`. Premi **Crea**.
3. Nella finestra che si apre premi **Scarica JSON**. Fallo subito: Google mostra il segreto del
   client solo in questo momento. Il file si chiama `client_secret_….apps.googleusercontent.com.json`.

## 5. In diMail

1. Al primo avvio diMail chiede il file: premi **Scegli il file JSON** oppure trascinalo sulla finestra.
2. Premi **Accedi con Google** e completa l'accesso nel browser.
   - Con un client Esterno compare "Google non ha verificato questa app": premi **Avanzate** e poi
     **Vai a diMail (non sicuro)**. È la tua app, sul tuo account.
   - Lascia selezionata la casella che permette a diMail di leggere e modificare la posta: senza,
     diMail non può funzionare e te lo dice.
3. Parte la prima sincronizzazione.

Il client resta nel Portachiavi del Mac, come il token di accesso. Per cambiarlo: Impostazioni →
Account Google → **Sostituisci il JSON** (con un client diverso dovrai rifare l'accesso; la posta già
scaricata resta).

## Problemi frequenti

- **"Accesso bloccato: l'amministratore ha limitato l'accesso"** (Workspace): l'organizzazione blocca
  le app di terze parti. L'amministratore apre la Console di amministrazione → Sicurezza → Accesso e
  controllo dei dati → Controlli API → Gestisci l'accesso delle app di terze parti, cerca il client
  per ID e lo segna come **Attendibile**.
- **"Questo è il JSON di un client «Applicazione web»"**: hai creato il client con il tipo sbagliato.
  Creane un altro di tipo **App desktop**.
- **Devi rifare l'accesso ogni settimana**: il client è Esterno e ancora in "Test". Pubblicalo (passo 3.5).
- **"Gmail API has not been used in project … or it is disabled"**: manca il passo 2.

## Un'azienda su Google Workspace

Con un client **Interno** basta farlo una volta per tutta l'azienda:

1. Un amministratore del Workspace (o chi può creare progetti Google Cloud nell'organizzazione) segue
   i passi da 1 a 4 e scarica il JSON.
2. Mette il file dove lo vedono solo i colleghi, per esempio in una cartella di Drive condivisa con
   l'azienda.
3. Ogni collega scarica diMail, al primo avvio sceglie quel file e preme **Accedi con Google** con
   il suo account aziendale.

Un client Interno non ha bisogno della verifica di Google e non mostra avvisi al login. Funziona
solo con gli account dell'organizzazione.

## E il segreto del client?

Per un'app desktop Google non considera davvero segreto il `client_secret`: chiunque abbia l'app può
leggerlo. A proteggere l'accesso sono il consenso nel browser e PKCE, che diMail usa sempre. Non
pubblicare comunque il file: identifica il tuo progetto Google Cloud.
