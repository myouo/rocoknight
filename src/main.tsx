import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";

class RootErrorBoundary extends React.Component<{ children: React.ReactNode }, { error?: Error }> {
  state: { error?: Error } = {};

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[RootErrorBoundary]", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="min-h-screen bg-ink text-mist p-6">
          <div className="text-lg font-display mb-2">RocoKnight failed to start</div>
          <div className="text-sm text-rose-200">
            {this.state.error.message || "Unknown error. Check the logs for details."}
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

const root = createRoot(document.getElementById("root")!);
root.render(
  <React.StrictMode>
    <RootErrorBoundary>
      <App />
    </RootErrorBoundary>
  </React.StrictMode>
);
