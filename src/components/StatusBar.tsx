import React from "react";
import clsx from "clsx";

export type AppStatus = "Login" | "Capturing" | "FoundValue" | "Launching" | "Running" | "Error";

interface StatusBarProps {
  status: AppStatus;
  message?: string | null;
  onRelogin: () => void;
  onCancel: () => void;
  onStop: () => void;
  onRestart: () => void;
}

const statusLabel: Record<AppStatus, string> = {
  Login: "Login",
  Capturing: "Capturing login3",
  FoundValue: "Found value",
  Launching: "Launching",
  Running: "Running",
  Error: "Error"
};

export default function StatusBar({
  status,
  message,
  onRelogin,
  onCancel,
  onStop,
  onRestart
}: StatusBarProps) {
  return (
    <div className="flex items-center justify-between px-6 py-4 border-b border-white/10 bg-steel/70 backdrop-blur">
      <div>
        <div className="font-display text-lg tracking-wide">RocoKnight</div>
        <div className="text-sm text-slate">
          Status
          <span
            className={clsx(
              "ml-2 inline-flex items-center px-2 py-0.5 rounded-full text-xs font-semibold",
              status === "Running" && "bg-emerald-500/15 text-emerald-200",
              status === "Error" && "bg-rose-500/20 text-rose-200",
              status === "Launching" && "bg-amber-500/20 text-amber-200",
              status === "FoundValue" && "bg-amber-500/20 text-amber-200",
              (status === "Login" || status === "Capturing") && "bg-sky-500/15 text-sky-200"
            )}
          >
            {statusLabel[status]}
          </span>
        </div>
        {message && <div className="text-xs text-rose-200/90 mt-1">{message}</div>}
      </div>
      <div className="flex items-center gap-2">
        <button
          className="px-3 py-1.5 rounded-md border border-white/15 text-sm hover:border-white/40 transition"
          onClick={onRelogin}
        >
          Retry / Re-login
        </button>
        <button
          className="px-3 py-1.5 rounded-md border border-white/15 text-sm hover:border-white/40 transition"
          onClick={onCancel}
        >
          Cancel
        </button>
        <button
          className="px-3 py-1.5 rounded-md border border-white/15 text-sm hover:border-white/40 transition"
          onClick={onStop}
        >
          Exit Game
        </button>
        <button
          className="px-3 py-1.5 rounded-md bg-ember/80 text-sm text-white hover:bg-ember transition"
          onClick={onRestart}
        >
          Restart
        </button>
      </div>
    </div>
  );
}
