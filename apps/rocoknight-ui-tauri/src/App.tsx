import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "./components/ui/button";

type CoreConfig = {
  launcher: {
    projector_path?: string | null;
    allow_multi_instance: boolean;
    auto_restart_on_crash: boolean;
  };
};

type LoginStatus = "Login" | "Waiting" | "Launching" | "Running" | "Error";

type EmbedRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export default function App() {
  const [cfg, setCfg] = useState<CoreConfig | null>(null);
  const [projector, setProjector] = useState("");
  const [status, setStatus] = useState<LoginStatus>("Login");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [debugLogs, setDebugLogs] = useState<string[]>([]);
  const [gameFrameStyle, setGameFrameStyle] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const loginFrameRef = useRef<HTMLDivElement>(null);
  const gameFrameRef = useRef<HTMLDivElement>(null);

  const statusLabel: Record<LoginStatus, string> = {
    Login: "Login",
    Waiting: "Waiting for Login",
    Launching: "Launching",
    Running: "Running",
    Error: "Error"
  };

  const gameStatusClass = useMemo(() => {
    if (status === "Running") return "text-emerald-300";
    if (status === "Error") return "text-red-300";
    if (status === "Launching") return "text-amber-200";
    return "text-white/70";
  }, [status]);

  useEffect(() => {
    invoke<CoreConfig>("get_config").then((data) => {
      setCfg(data);
      setProjector(data.launcher.projector_path ?? "");
    });

    const unlistenStatus = listen<{ status: LoginStatus }>("login_status", (event) => {
      setStatus(event.payload.status);
      if (event.payload.status === "Running") {
        setRunning(true);
      }
      if (event.payload.status === "Login" || event.payload.status === "Error") {
        setRunning(false);
      }
    });
    const unlistenError = listen<{ message: string }>("login_error", (event) => {
      setError(event.payload.message);
    });
    const unlistenDebug = listen<{ message: string }>("login_debug", (event) => {
      setDebugLogs((prev) => [event.payload.message, ...prev].slice(0, 20));
    });

    return () => {
      unlistenStatus.then((fn) => fn());
      unlistenError.then((fn) => fn());
      unlistenDebug.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const updateRects = () => {
      if (!stageRef.current || !loginFrameRef.current || !gameFrameRef.current) return;
      const stage = stageRef.current.getBoundingClientRect();
      const loginRect: EmbedRect = {
        x: Math.round(stage.left),
        y: Math.round(stage.top),
        width: Math.round(stage.width),
        height: Math.round(stage.height)
      };
      invoke("set_login_rect", { rect: loginRect }).catch(() => undefined);

      const aspectW = 12;
      const aspectH = 7;
      const containerW = stage.width;
      const containerH = stage.height;
      let width = containerW;
      let height = (containerW * aspectH) / aspectW;
      if (height > containerH) {
        height = containerH;
        width = (containerH * aspectW) / aspectH;
      }
      const offsetX = (containerW - width) / 2;
      const offsetY = (containerH - height) / 2;
      const left = stage.left + offsetX;
      const top = stage.top + offsetY;

      const rect: EmbedRect = {
        x: Math.round(left),
        y: Math.round(top),
        width: Math.round(width),
        height: Math.round(height)
      };
      invoke("set_game_rect", { rect }).catch(() => undefined);
      setGameFrameStyle({
        left: Math.round(offsetX),
        top: Math.round(offsetY),
        width: Math.round(width),
        height: Math.round(height)
      });
    };

    const observer = new ResizeObserver(() => updateRects());
    if (stageRef.current) observer.observe(stageRef.current);
    window.addEventListener("resize", updateRects);
    updateRects();

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", updateRects);
    };
  }, []);

  useEffect(() => {
    if (status === "Login") {
      invoke("start_login_flow").catch((e) => setError(String(e)));
    }
  }, [status]);

  const saveConfig = async () => {
    if (!cfg) return;
    const next = {
      ...cfg,
      launcher: {
        ...cfg.launcher,
        projector_path: projector || null
      }
    };
    await invoke("set_config", { cfg: next });
    setCfg(next);
  };

  const pickProjector = async () => {
    const selected = await open({
      title: "选择 Flash Projector",
      multiple: false,
      filters: [{ name: "Executable", extensions: ["exe"] }]
    });
    if (typeof selected === "string") {
      setProjector(selected);
      setError(null);
      await saveConfig();
    }
  };

  const retryLogin = async () => {
    setError(null);
    await saveConfig();
    await invoke("start_login_flow");
  };

  const stopGame = async () => {
    setError(null);
    await invoke("stop_game");
    setStatus("Login");
  };

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,_#1b1f2a,_#0a0c10_60%)] text-white">
      <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-8 py-6">
        <header className="flex items-center justify-between border-b border-white/10 pb-4">
          <div>
            <p className="text-xs uppercase tracking-[0.2em] text-white/40">RocoKnight Launcher</p>
            <h1 className="mt-2 font-display text-3xl font-semibold">洛克王国 · 一体化登录与运行</h1>
          </div>
          <div className="flex items-center gap-3">
            <div className={`rounded-full border border-white/10 bg-white/5 px-4 py-2 text-xs ${gameStatusClass}`}>
              状态: {statusLabel[status]}
            </div>
            <Button variant="ghost" onClick={stopGame} className="h-9 px-4">退出游戏</Button>
          </div>
        </header>

        <main className="mt-6 grid flex-1 gap-6 lg:grid-cols-[1fr_320px]">
          <section className="relative overflow-hidden rounded-2xl border border-white/10 bg-white/5">
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-3">
              <h2 className="text-sm font-semibold text-white/80">GameContainer</h2>
              <div className="text-xs text-white/50">基准 960×560 (12:7)</div>
            </div>
            <div ref={stageRef} className="relative h-[560px] bg-black/40">
              <div
                ref={loginFrameRef}
                className="absolute inset-0 flex items-center justify-center bg-black/30"
                style={{ opacity: status === "Running" ? 0 : 1, pointerEvents: status === "Running" ? "none" : "auto" }}
              >
                <div className="text-center">
                  <p className="text-sm text-white/70">登录页面加载中</p>
                  <p className="mt-2 text-xs text-white/40">请在窗口内完成登录</p>
                </div>
              </div>
              <div
                ref={gameFrameRef}
                className="absolute"
                style={{
                  opacity: status === "Running" ? 1 : 0,
                  pointerEvents: status === "Running" ? "auto" : "none",
                  left: gameFrameStyle?.left ?? 0,
                  top: gameFrameStyle?.top ?? 0,
                  width: gameFrameStyle?.width ?? "100%",
                  height: gameFrameStyle?.height ?? "100%"
                }}
              />
              {status === "Running" ? null : (
                <div className="pointer-events-none absolute inset-0 border border-white/10" />
              )}
            </div>
          </section>

          <section className="rounded-2xl border border-white/10 bg-white/5 p-6">
            <h2 className="text-lg font-semibold">控制面板</h2>
            <div className="mt-4 rounded-xl border border-white/10 bg-black/30 p-4 text-sm">
              <p className="text-white/70">运行状态</p>
              <p className="mt-2 text-2xl font-semibold">
                {running ? "运行中" : "未启动"}
              </p>
              <p className="mt-2 text-xs text-white/50">登录状态: {statusLabel[status]}</p>
            </div>

            <div className="mt-5 space-y-3">
              <Button onClick={retryLogin} className="h-10 w-full">重新登录</Button>
              <Button variant="ghost" onClick={stopGame} className="h-10 w-full">退出游戏/返回登录</Button>
            </div>

            {error && <p className="mt-4 text-sm text-red-400">{error}</p>}

            <div className="mt-6 border-t border-white/10 pt-4">
              <h3 className="text-sm font-semibold text-white/80">Projector 设置</h3>
              <div className="mt-2 text-xs text-white/50">默认使用内置 projector.exe</div>
              <div className="mt-3 flex gap-3">
                <input
                  className="w-full rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-xs outline-none focus:border-amber-400"
                  value={projector}
                  onChange={(e) => setProjector(e.target.value)}
                  placeholder="可选自定义路径"
                />
                <Button variant="ghost" onClick={pickProjector} className="h-9 px-4">
                  选择
                </Button>
              </div>
            </div>

            <div className="mt-6 border-t border-white/10 pt-4">
              <h3 className="text-sm font-semibold text-white/80">调试日志</h3>
              <div className="mt-2 h-40 overflow-auto rounded-xl border border-white/10 bg-black/30 p-3 text-xs text-white/60">
                {debugLogs.length === 0 ? "暂无日志" : debugLogs.map((line, idx) => (
                  <div key={idx}>{line}</div>
                ))}
              </div>
            </div>
          </section>
        </main>
      </div>
    </div>
  );
}
