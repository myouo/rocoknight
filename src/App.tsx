import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrent } from "@tauri-apps/api/window";
import GameView from "./components/GameView";
import LoginView from "./components/LoginView";
import StatusBar, { AppStatus } from "./components/StatusBar";

interface StatusPayload {
  status: AppStatus;
  message?: string | null;
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

const BASE_W = 960;
const BASE_H = 560;
const BASE_RATIO = BASE_W / BASE_H;
const ROCO_GREEN = "color: #2ecc71; font-weight: 600;";
const ROCO_BLUE = "color: #3498db; font-weight: 600;";
const ROCO_RESET = "color: inherit;";

function logUi(...args: unknown[]) {
  console.log("%c[RocoKnight]%c[ui]%c", ROCO_GREEN, ROCO_BLUE, ROCO_RESET, ...args);
}

function fitRect(container: DOMRect): Rect {
  const containerRatio = container.width / container.height;
  let w = container.width;
  let h = container.height;
  if (containerRatio > BASE_RATIO) {
    h = container.height;
    w = h * BASE_RATIO;
  } else {
    w = container.width;
    h = w / BASE_RATIO;
  }
  const x = container.left + (container.width - w) / 2;
  const y = container.top + (container.height - h) / 2;
  return { x, y, w, h };
}

function useAspectFrame(
  stageRef: React.RefObject<HTMLDivElement>,
  onChange: (rect: Rect) => void
) {
  const [style, setStyle] = useState<React.CSSProperties>({});

  useLayoutEffect(() => {
    if (!stageRef.current) return;

    const update = () => {
      const el = stageRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const fitted = fitRect(rect);
      setStyle({
        width: `${fitted.w}px`,
        height: `${fitted.h}px`,
        left: `${fitted.x - rect.left}px`,
        top: `${fitted.y - rect.top}px`
      });
      onChange(fitted);
    };

    update();

    const resizeObserver = new ResizeObserver(update);
    resizeObserver.observe(stageRef.current);
    window.addEventListener("resize", update);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [stageRef, onChange]);

  return style;
}

export default function App() {
  const [status, setStatus] = useState<AppStatus>("Login");
  const [message, setMessage] = useState<string | null>(null);
  const [fatalError, setFatalError] = useState<string | null>(null);
  const loginStageRef = useRef<HTMLDivElement>(null);
  const loginFrameRef = useRef<HTMLDivElement>(null);
  const gameStageRef = useRef<HTMLDivElement>(null);
  const gameFrameRef = useRef<HTMLDivElement>(null);
  const isTauri =
    typeof (window as any).__TAURI__ !== "undefined" ||
    typeof (window as any).__TAURI_INTERNALS__ !== "undefined";

  const isGameView = status === "Running" || status === "Launching" || status === "FoundValue";

  const reportError = useCallback((context: string, error: unknown) => {
    const detail = error instanceof Error ? error.message : String(error);
    const msg = `${context} failed: ${detail}`;
    console.error(msg, error);
    setStatus("Error");
    setMessage(msg);
    setFatalError(msg);
  }, []);

  const safeInvoke = useCallback(
    async <T,>(cmd: string, payload?: Record<string, unknown>) => {
      if (!isTauri) {
        return null as T | null;
      }
      try {
        return (await invoke<T>(cmd, payload ?? {})) ?? null;
      } catch (error) {
        reportError(`invoke:${cmd}`, error);
        return null as T | null;
      }
    },
    [isTauri, reportError]
  );

  const getScaleFactorSafe = useCallback(async () => {
    if (!isTauri) return 1;
    try {
      return await getCurrent().scaleFactor();
    } catch (error) {
      reportError("scaleFactor", error);
      return 1;
    }
  }, [isTauri, reportError]);

  const updateLoginBounds = useCallback(
    async (rect: Rect) => {
      const logical = {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        w: Math.round(rect.w),
        h: Math.round(rect.h)
      };
      await safeInvoke("set_login_bounds", { rect: logical });
    },
    [safeInvoke]
  );

  const updateProjectorBounds = useCallback(
    async (rect: Rect) => {
      const scale = await getScaleFactorSafe();
      const physical = {
        x: Math.round(rect.x * scale),
        y: Math.round(rect.y * scale),
        w: Math.round(rect.w * scale),
        h: Math.round(rect.h * scale)
      };
      await safeInvoke("resize_projector", { rect: physical });
    },
    [getScaleFactorSafe, safeInvoke]
  );

  const loginFrameStyle = useAspectFrame(loginStageRef, (rect) => {
    if (!isGameView) {
      updateLoginBounds(rect);
    }
  });

  const gameFrameStyle = useAspectFrame(gameStageRef, (rect) => {
    if (isGameView) {
      updateProjectorBounds(rect);
    }
  });

  useEffect(() => {
    const unlistenPromise = listen<StatusPayload>("status_changed", (event) => {
      logUi("status_changed", event.payload.status, event.payload.message ?? "");
      setStatus(event.payload.status);
      setMessage(event.payload.message ?? null);
      if (event.payload.status !== "Error") {
        setFatalError(null);
      }
    }).catch((error) => {
      reportError("listen:status_changed", error);
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, [isTauri, reportError]);

  useEffect(() => {
    const init = async () => {
      await safeInvoke("show_login_webview");
      await safeInvoke("start_login3_capture");
    };
    init();
  }, [safeInvoke]);

  useEffect(() => {
    const launch = async () => {
      if (status !== "Launching" && status !== "FoundValue") return;
      logUi("launch_projector invoke");
      await safeInvoke("hide_login_webview");
      const rect = gameFrameRef.current?.getBoundingClientRect();
      if (!rect) return;
      const scale = await getScaleFactorSafe();
      const physical = {
        x: Math.round(rect.left * scale),
        y: Math.round(rect.top * scale),
        w: Math.round(rect.width * scale),
        h: Math.round(rect.height * scale)
      };
      await safeInvoke("launch_projector", { rect: physical });
    };
    launch();
  }, [status, getScaleFactorSafe, safeInvoke]);

  const handleRelogin = async () => {
    await safeInvoke("reset_to_login");
    await safeInvoke("show_login_webview");
    await safeInvoke("start_login3_capture");
  };

  const handleCancel = async () => {
    await safeInvoke("stop_login3_capture");
    await safeInvoke("show_login_webview");
  };

  const handleStop = async () => {
    await safeInvoke("stop_projector");
    await safeInvoke("show_login_webview");
    await safeInvoke("start_login3_capture");
  };

  const handleRestart = async () => {
    const rect = gameFrameRef.current?.getBoundingClientRect();
    if (!rect) return;
    const scale = await getScaleFactorSafe();
    const physical = {
      x: Math.round(rect.left * scale),
      y: Math.round(rect.top * scale),
      w: Math.round(rect.width * scale),
      h: Math.round(rect.height * scale)
    };
    await safeInvoke("restart_projector", { rect: physical });
  };

  return (
    <div className="app-shell">
      <div className="px-6 py-2 text-xs text-mist/70">RocoKnight</div>
      {fatalError && <div className="px-6 pb-2 text-xs text-rose-200">{fatalError}</div>}
      <StatusBar
        status={status}
        message={message}
        onRelogin={handleRelogin}
        onCancel={handleCancel}
        onStop={handleStop}
        onRestart={handleRestart}
      />
      {isGameView ? (
        <GameView stageRef={gameStageRef} frameRef={gameFrameRef} frameStyle={gameFrameStyle} />
      ) : (
        <LoginView stageRef={loginStageRef} frameRef={loginFrameRef} frameStyle={loginFrameStyle} />
      )}
    </div>
  );
}
