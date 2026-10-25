
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type MidiPort = {
  id: string;
  name: string;
};

type ServiceStatus = {
  available: boolean;
  message: string;
  ports: MidiPort[];
  backend: MidiBackend;
};

type MidiBackend = "teVirtualMidi" | "windowsMidiServices";

function errorMessage(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "The MIDI host could not be reached.";
}

function App() {
  const [ports, setPorts] = useState<MidiPort[]>([]);
  const [backend, setBackend] = useState<MidiBackend>("teVirtualMidi");
  const [serviceReady, setServiceReady] = useState(false);
  const [serviceLoading, setServiceLoading] = useState(true);
  const [serviceMessage, setServiceMessage] = useState("Checking MIDI backend…");
  const [newPortName, setNewPortName] = useState("");
  const [showInput, setShowInput] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    void invoke<ServiceStatus>("service_status")
      .then((status) => {
        setBackend(status.backend);
        setServiceReady(status.available);
        setServiceMessage(status.message);
        setPorts(status.ports);
      })
      .catch((reason: unknown) => {
        setServiceReady(false);
        setServiceMessage(errorMessage(reason));
      })
      .finally(() => setServiceLoading(false));
  }, []);

  async function chooseBackend(nextBackend: MidiBackend) {
    if (busy || serviceLoading || backend === nextBackend) return;
    setBusy(true);
    setError("");
    try {
      const status = await invoke<ServiceStatus>("set_backend", { backend: nextBackend });
      setBackend(status.backend);
      setServiceReady(status.available);
      setServiceMessage(status.message);
      setPorts(status.ports);
      setShowInput(false);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  const backendLabel = backend === "teVirtualMidi" ? "teVirtualMIDI" : "Windows MIDI Services";

  async function createPort() {
    const name = newPortName.trim();
    if (!name || busy) return;
    setBusy(true);
    setError("");
    try {
      const port = await invoke<MidiPort>("create_port", { name });
      setPorts((current) => [...current, port]);
      setNewPortName("");
      setShowInput(false);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  async function deletePort(id: string) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await invoke("delete_port", { id });
      setPorts((current) => current.filter((port) => port.id !== id));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark">
            <span />
            <span />
            <span />
          </div>
          <span>midi<span className="brand-accent">-fiesta</span></span>
          <span className="version">BETA</span>
        </div>
        <div className="system-status">
          <span className={`status-dot ${serviceLoading ? "is-loading" : serviceReady ? "is-ready" : "is-offline"}`} />
          {serviceLoading ? "CHECKING MIDI" : serviceReady ? `${backend === "teVirtualMidi" ? "TE VIRTUAL MIDI" : "MIDI SERVICES"} READY` : `${backend === "teVirtualMidi" ? "TE VIRTUAL MIDI" : "MIDI SERVICES"} OFF`}
        </div>
      </header>

      <section className="main-content">
        <div className="eyebrow">
          <span className="eyebrow-line" />
          <span>CHOOSE YOUR PROVIDER:</span>
          <span className="eyebrow-line" />
        </div>

        <div className="backend-picker" aria-label="MIDI backend">
          <button
            className={`backend-choice ${backend === "teVirtualMidi" ? "is-selected" : ""}`}
            onClick={() => chooseBackend("teVirtualMidi")}
            disabled={busy || serviceLoading}
            aria-pressed={backend === "teVirtualMidi"}
          >
            <span>teVirtualMIDI</span>
            <small>Uses installed teVirtualMIDI driver</small>
          </button>
          <button
            className={`backend-choice ${backend === "windowsMidiServices" ? "is-selected" : ""}`}
            onClick={() => chooseBackend("windowsMidiServices")}
            disabled={busy || serviceLoading}
            aria-pressed={backend === "windowsMidiServices"}
          >
            <span>Windows MIDI Services</span>
            <small>Windows 11 25H2+</small>
          </button>
        </div>

        <div className="hero">
          <h1>Your MIDI.<br /><span>To rule them all.</span></h1>
          <p>
            Create how many you want.
            <br />
            One simple space for all your connections.
          </p>
        </div>

        <div className="section-heading">
          <div>
            <h2>Your ports <span className="count">{ports.length}</span></h2>
            <p className="section-description">Manage your virtual MIDI endpoints</p>
          </div>
          <button className="create-button" onClick={() => { setError(""); setShowInput(true); }} disabled={!serviceReady || serviceLoading || busy}>
            <span className="plus">+</span> Create port
          </button>
        </div>

        {showInput && (
          <form
            className="new-port-form"
            onSubmit={(event) => {
              event.preventDefault();
              createPort();
            }}
          >
            <input
              autoFocus
              value={newPortName}
              onChange={(event) => setNewPortName(event.target.value)}
              placeholder="Name your MIDI port..."
              aria-label="MIDI port name"
              maxLength={31}
            />
            <button className="confirm-button" type="submit" disabled={busy || !newPortName.trim()}>{busy ? "Creating…" : "Add port"}</button>
            <button
              className="cancel-button"
              type="button"
              disabled={busy}
              onClick={() => {
                setShowInput(false);
                setNewPortName("");
              }}
            >
              Cancel
            </button>
          </form>
        )}

        <div className="port-list">
          {ports.map((port, index) => (
            <article className="port-card" key={port.id}>
              <div className="port-icon">
                <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
                  <rect x="3.5" y="5" width="17" height="14" rx="3" />
                  <path d="M8 9v6M12 9v6M16 9v6M6 12h12" />
                </svg>
              </div>

              <div className="port-info">
                <h3>{port.name}</h3>
                <p><span className="mini-dot" /> Temporary virtual port</p>
              </div>

              <span className="port-number">
                {String(index + 1).padStart(2, "0")}
              </span>

              <button
                className="delete-button"
                onClick={() => deletePort(port.id)}
                disabled={busy}
                aria-label={`Remove ${port.name}`}
                title="Remove port"
              >
                ×
              </button>
            </article>
          ))}

          {ports.length === 0 && (
            <div className="empty-state">
              <div className="empty-symbol">＋</div>
              <h3>No ports yet</h3>
              <p>Create your first port to get started.</p>
            </div>
          )}
        </div>

        {(error || !serviceReady || serviceLoading) && <div className={`notice ${error ? "notice-error" : serviceReady ? "notice-ready" : ""}`}>
          <span className="notice-icon">{error ? "!" : "i"}</span>
          <p>
            <strong>{error ? "Could not complete that action" : serviceLoading ? "Checking MIDI support" : serviceReady ? `${backendLabel} ready` : `${backendLabel} unavailable`}</strong>
            <span>{error || serviceMessage}</span>
          </p>
        </div>}
      </section>

      <footer className="footer">
        <span>2026 © by meiv</span>
        <span>midi-fiesta <span className="footer-separator">/</span> 0.1.0</span>
      </footer>
    </main>
  );
}

export default App;
