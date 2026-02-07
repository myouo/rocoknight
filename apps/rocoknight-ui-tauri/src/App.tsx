import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "./components/ui/button";

type CoreConfig = {
  launcher: {
    projector_path?: string | null;
    main_swf_url?: string | null;
    allow_multi_instance: boolean;
    auto_restart_on_crash: boolean;
  };
};

type ProcessHandle = { id: number };
type LoginStatus = "Idle" | "WaitingForLogin" | "Launching" | "Running";

export default function App() {
  const [cfg, setCfg] = useState<CoreConfig | null>(null);
  const [url, setUrl] = useState("");
  const [projector, setProjector] = useState("");
  const [handle, setHandle] = useState<ProcessHandle | null>(null);
  const [running, setRunning] = useState(false);
  const [loginStatus, setLoginStatus] = useState<LoginStatus>("Idle");
  const [error, setError] = useState<string | null>(null);
  const statusLabel: Record<LoginStatus, string> = {
    Idle: "Idle",
    WaitingForLogin: "Waiting for Login",
    Launching: "Launching",
    Running: "Running"
  };

  useEffect(() => {
    invoke<CoreConfig>("get_config").then((data) => {
      setCfg(data);
      setUrl(data.launcher.main_swf_url ?? "");
      setProjector(data.launcher.projector_path ?? "");
    });

    const unlistenStatus = listen<{ status: LoginStatus }>("login_status", (event) => {
      setLoginStatus(event.payload.status);
      if (event.payload.status === "Running") {
        setRunning(true);
      }
      if (event.payload.status === "Idle") {
        setRunning(false);
      }
    });
    const unlistenError = listen<{ message: string }>("login_error", (event) => {
      setError(event.payload.message);
    });

    return () => {
      unlistenStatus.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, []);

  const saveConfig = async () => {
    if (!cfg) return;
    const next = {
      ...cfg,
      launcher: {
        ...cfg.launcher,
        main_swf_url: url,
        projector_path: projector
      }
    };
    await invoke("set_config", { cfg: next });
    setCfg(next);
  };

  const pickProjector = async () => {
    const selected = await open({
      title: "选择 Flash Projector",
      multiple: false,
      filters: [
        { name: "Executable", extensions: ["exe"] }
      ]
    });
    if (typeof selected === "string") {
      setProjector(selected);
    }
  };

  const launch = async () => {
    setError(null);
    try {
      await saveConfig();
      const h = await invoke<ProcessHandle>("launch");
      setHandle(h);
      setRunning(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const loginAndLaunch = async () => {
    setError(null);
    try {
      await saveConfig();
      await invoke("login_and_launch");
    } catch (e) {
      setError(String(e));
    }
  };

  const stop = async () => {
    if (!handle) return;
    setError(null);
    try {
      await invoke("stop", { handle });
      setRunning(false);
    } catch (e) {
      setError(String(e));
    }
  };

  const refreshStatus = async () => {
    if (!handle) return;
    const isRunning = await invoke<boolean>("is_running", { handle });
    setRunning(isRunning);
  };

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,_#1b1f2a,_#0a0c10_60%)] text-white">
      <div className="mx-auto flex min-h-screen max-w-5xl flex-col px-8 py-10">
        <header className="flex items-center justify-between">
          <div>
            <p className="text-xs uppercase tracking-[0.2em] text-white/40">RocoKnight Launcher</p>
            <h1 className="mt-2 font-display text-4xl font-semibold">高效稳定的洛克王国启动器</h1>
          </div>
          <div className="rounded-full border border-white/10 bg-white/5 px-4 py-2 text-xs text-white/70">
            Windows 10/11
          </div>
        </header>

        <main className="mt-10 grid gap-6 lg:grid-cols-[1.2fr_0.8fr]">
          <section className="rounded-2xl border border-white/10 bg-white/5 p-6 shadow-[0_20px_60px_-40px_rgba(255,255,255,0.45)]">
            <h2 className="text-lg font-semibold">启动配置</h2>
            <p className="mt-1 text-sm text-white/60">
              输入 `main.swf` 的完整 URL，并配置本机 Flash Standalone Projector 路径。
            </p>
            <div className="mt-6 space-y-4">
              <label className="block text-sm text-white/70">main.swf URL</label>
              <input
                className="w-full rounded-lg border border-white/10 bg-black/30 px-4 py-3 text-sm outline-none focus:border-amber-400"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://.../main.swf"
              />
              <label className="block text-sm text-white/70">Projector 路径</label>
              <div className="flex gap-3">
                <input
                  className="w-full rounded-lg border border-white/10 bg-black/30 px-4 py-3 text-sm outline-none focus:border-amber-400"
                  value={projector}
                  onChange={(e) => setProjector(e.target.value)}
                  placeholder="C:\\Path\\to\\FlashPlayer.exe"
                />
                <Button variant="ghost" onClick={pickProjector} className="h-11 px-4">
                  选择
                </Button>
              </div>
            </div>
            <div className="mt-6 flex gap-3">
              <Button onClick={launch} className="h-11 px-6">一键启动</Button>
              <Button onClick={loginAndLaunch} className="h-11 px-6">Login & Launch / 登录并启动</Button>
              <Button variant="ghost" onClick={refreshStatus} className="h-11 px-6">刷新状态</Button>
              <Button variant="ghost" onClick={stop} className="h-11 px-6">停止</Button>
            </div>
            {error && <p className="mt-4 text-sm text-red-400">{error}</p>}
          </section>

          <section className="rounded-2xl border border-white/10 bg-white/5 p-6">
            <h2 className="text-lg font-semibold">运行状态</h2>
            <div className="mt-4 rounded-xl border border-white/10 bg-black/30 p-4">
              <p className="text-sm text-white/60">进程状态</p>
              <p className="mt-2 text-2xl font-semibold">
                {running ? "运行中" : "未启动"}
              </p>
              <p className="mt-2 text-xs text-white/50">登录状态: {statusLabel[loginStatus]}</p>
              {handle && <p className="mt-2 text-xs text-white/50">PID: {handle.id}</p>}
            </div>
            <div className="mt-6">
              <h3 className="text-sm font-semibold text-white/80">日志</h3>
              <div className="mt-2 h-40 rounded-xl border border-white/10 bg-black/30 p-4 text-xs text-white/50">
                核心日志将在这里显示（下一阶段接入 tracing + 文件输出）。
              </div>
            </div>
          </section>
        </main>
      </div>
    </div>
  );
}
