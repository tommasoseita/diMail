// Un errore durante il disegno non deve lasciare la finestra bianca: si mostra, finisce nel log e
// si può ricaricare l'interfaccia. La sync va avanti comunque, è nel nucleo.
import { Component, type ErrorInfo, type ReactNode } from "react";
import { describe, report } from "./diagnostics";

type State = { error: Error | null };

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    report("error", `errore nel disegno: ${describe(error)}\ncomponenti:${info.componentStack ?? ""}`);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="center">
        <div className="login">
          <h1>Qualcosa si è rotto</h1>
          <p className="muted">L'errore è nel log di diMail. La posta e la sync non ne risentono.</p>
          <pre className="plain small error">{this.state.error.message}</pre>
          <button className="primary" onClick={() => location.reload()}>
            Ricarica l'interfaccia
          </button>
        </div>
      </div>
    );
  }
}
