import React from "react";
import ReactDOM from "react-dom/client";
import { initServiceWorker } from "./lib/sw-register";
import App from "./App";
import "./index.css";

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("LocalMind boot error:", error, info);
  }
  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 24, color: "#ef4444", fontFamily: "monospace", whiteSpace: "pre-wrap", background: "#0a0a0b", minHeight: "100vh" }}>
          <h2 style={{ marginTop: 0 }}>LocalMind crashed on boot</h2>
          <p>{this.state.error.message}</p>
          <pre style={{ fontSize: 12, color: "#9a9aa3" }}>{this.state.error.stack}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);

initServiceWorker();
