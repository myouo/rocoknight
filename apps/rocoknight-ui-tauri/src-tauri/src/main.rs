#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rocoknight_core::{
    CoreConfig, EmbedRect, ProcessHandle, ProcessManager, RawHwnd,
    window_embed::{attach_child, detach, find_window_by_pid, set_child_rect},
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder};

type SharedConfig = Arc<Mutex<CoreConfig>>;
type SharedEmbed = Arc<EmbedState>;

#[derive(Default)]
struct EmbedState {
    login_in_progress: Mutex<bool>,
    login_hwnd: Mutex<Option<RawHwnd>>,
    login_old_style: Mutex<Option<isize>>,
    projector_hwnd: Mutex<Option<RawHwnd>>,
    projector_old_style: Mutex<Option<isize>>,
    projector_handle: Mutex<Option<ProcessHandle>>,
    login_rect: Mutex<Option<EmbedRect>>,
    game_rect: Mutex<Option<EmbedRect>>,
}

#[derive(Clone, serde::Serialize)]
struct StatusPayload {
    status: &'static str,
}

#[derive(Clone, serde::Serialize)]
struct ErrorPayload {
    message: String,
}

#[derive(Clone, serde::Serialize)]
struct DebugPayload {
    message: String,
}

#[tauri::command]
fn get_config(state: tauri::State<SharedConfig>) -> CoreConfig {
    state.lock().unwrap().clone()
}

#[tauri::command]
fn set_config(state: tauri::State<SharedConfig>, cfg: CoreConfig) {
    *state.lock().unwrap() = cfg;
}

#[tauri::command]
fn is_running(manager: tauri::State<ProcessManager>, state: tauri::State<SharedEmbed>) -> bool {
    if let Some(handle) = state.projector_handle.lock().unwrap().clone() {
        manager.is_running(&handle)
    } else {
        false
    }
}

#[tauri::command]
fn set_login_rect(state: tauri::State<SharedEmbed>, rect: EmbedRect) -> Result<(), String> {
    *state.login_rect.lock().unwrap() = Some(rect);
    if let Some(hwnd) = *state.login_hwnd.lock().unwrap() {
        let _ = set_child_rect(hwnd, rect).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn set_game_rect(state: tauri::State<SharedEmbed>, rect: EmbedRect) -> Result<(), String> {
    *state.game_rect.lock().unwrap() = Some(rect);
    if let Some(hwnd) = *state.projector_hwnd.lock().unwrap() {
        let _ = set_child_rect(hwnd, rect).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn start_login_flow(
    app: AppHandle,
    state: tauri::State<'_, SharedConfig>,
    manager: tauri::State<'_, ProcessManager>,
    embed: tauri::State<'_, SharedEmbed>,
) -> Result<(), String> {
    {
        let mut guard = embed.login_in_progress.lock().unwrap();
        if *guard {
            return Err("login already in progress".to_string());
        }
        *guard = true;
    }

    let _ = app.emit(
        "login_status",
        StatusPayload {
            status: "Waiting",
        },
    );
    let _ = app.emit(
        "login_debug",
        DebugPayload {
            message: "开始登录流程".to_string(),
        },
    );

    if let Some(existing) = app.get_webview_window("login") {
        let _ = existing.close();
    }

    let login_url = "https://17roco.qq.com/login.html";
    let window = WebviewWindowBuilder::new(&app, "login", WebviewUrl::External(login_url.parse().unwrap()))
        .title("RocoKnight Login")
        .inner_size(900.0, 720.0)
        .resizable(true)
        .visible(true)
        .build()
        .map_err(|e| {
            let mut guard = embed.login_in_progress.lock().unwrap();
            *guard = false;
            e.to_string()
        })?;

    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let main_hwnd = main_window.hwnd().map_err(|e| e.to_string())?.0 as RawHwnd;
    let login_hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as RawHwnd;

    let old_style = attach_child(main_hwnd, login_hwnd).map_err(|e| e.to_string())?;
    *embed.login_hwnd.lock().unwrap() = Some(login_hwnd);
    *embed.login_old_style.lock().unwrap() = Some(old_style);
    if let Some(rect) = *embed.login_rect.lock().unwrap() {
        let _ = set_child_rect(login_hwnd, rect).map_err(|e| e.to_string())?;
    }

    let (close_tx, mut close_rx) = tokio::sync::watch::channel::<bool>(false);
    let finished = Arc::new(AtomicBool::new(false));
    let finished_close = finished.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            if !finished_close.load(Ordering::SeqCst) {
                let _ = close_tx.send(true);
            }
        }
    });

    let app_clone = app.clone();
    let cfg = state.lock().unwrap().clone();
    let manager = manager.inner().clone();
    let embed_state = embed.inner().clone();
    let finished_task = finished.clone();

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(200));
        let timeout = Duration::from_secs(180);
        let start = tokio::time::Instant::now();

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if start.elapsed() > timeout {
                        finished_task.store(true, Ordering::SeqCst);
                        let _ = window.close();
                        let _ = app_clone.emit("login_error", ErrorPayload {
                            message: "登录超时，请重试".to_string(),
                        });
                        let _ = app_clone.emit("login_debug", DebugPayload {
                            message: "登录超时，未捕获到 main.swf".to_string(),
                        });
                        let _ = app_clone.emit("login_status", StatusPayload { status: "Error" });
                        break;
                    }

                    if let Ok(current) = window.url() {
                        if let Some(matched_url) = match_main_swf(&current) {
                            let _ = app_clone.emit("login_status", StatusPayload { status: "Launching" });
                            let _ = app_clone.emit("login_debug", DebugPayload {
                                message: format!("捕获到目标 URL: {}", redact_url(&matched_url)),
                            });
                            finished_task.store(true, Ordering::SeqCst);
                            let _ = window.close();

                            match launch_and_embed(&app_clone, &manager, &embed_state, &cfg, matched_url).await {
                                Ok(_) => {
                                    let _ = app_clone.emit("login_status", StatusPayload { status: "Running" });
                                    let _ = app_clone.emit("login_debug", DebugPayload {
                                        message: "Projector 启动并嵌入成功".to_string(),
                                    });
                                }
                                Err(message) => {
                                    let _ = app_clone.emit("login_error", ErrorPayload { message });
                                    let _ = app_clone.emit("login_status", StatusPayload { status: "Error" });
                                }
                            }
                            break;
                        }
                    }
                }
                changed = close_rx.changed() => {
                    if changed.is_ok() && *close_rx.borrow() {
                        finished_task.store(true, Ordering::SeqCst);
                        let _ = app_clone.emit("login_error", ErrorPayload {
                            message: "登录窗口已关闭".to_string(),
                        });
                        let _ = app_clone.emit("login_debug", DebugPayload {
                            message: "登录窗口被用户关闭".to_string(),
                        });
                        let _ = app_clone.emit("login_status", StatusPayload { status: "Error" });
                        break;
                    }
                }
            }
        }

        let mut guard = embed_state.login_in_progress.lock().unwrap();
        *guard = false;
        *embed_state.login_hwnd.lock().unwrap() = None;
        *embed_state.login_old_style.lock().unwrap() = None;
    });

    Ok(())
}

#[tauri::command]
fn stop_game(
    app: AppHandle,
    manager: tauri::State<'_, ProcessManager>,
    embed: tauri::State<'_, SharedEmbed>,
) -> Result<(), String> {
    if let Some(handle) = embed.projector_handle.lock().unwrap().take() {
        let _ = manager.stop(&handle);
    }

    if let Some(hwnd) = embed.projector_hwnd.lock().unwrap().take() {
        if let Some(old_style) = embed.projector_old_style.lock().unwrap().take() {
            let _ = detach(hwnd, old_style);
        }
    }

    let _ = app.emit("login_status", StatusPayload { status: "Login" });
    Ok(())
}

fn match_main_swf(url: &Url) -> Option<String> {
    let raw = url.as_str();
    if !raw.contains("main.swf") {
        return None;
    }
    if let Some(host) = url.host_str() {
        if !host.ends_with("qq.com") {
            return None;
        }
    }
    Some(raw.to_string())
}

async fn launch_and_embed(
    app: &AppHandle,
    manager: &ProcessManager,
    embed: &SharedEmbed,
    cfg: &CoreConfig,
    swf_url: String,
) -> Result<(), String> {
    let projector_path = resolve_projector_path(app, cfg)?;
    let handle = manager
        .launch_projector_with_url(projector_path, swf_url)
        .map_err(|e| format!("启动失败: {}", e))?;

    *embed.projector_handle.lock().unwrap() = Some(handle.clone());

    let hwnd = find_window_by_pid(handle.pid, Duration::from_secs(10))
        .map_err(|_| "未找到 Projector 窗口".to_string())?;

    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let parent_hwnd = main_window.hwnd().map_err(|e| e.to_string())?.0 as RawHwnd;

    let old_style = attach_child(parent_hwnd, hwnd).map_err(|_| "嵌入窗口失败".to_string())?;
    *embed.projector_hwnd.lock().unwrap() = Some(hwnd);
    *embed.projector_old_style.lock().unwrap() = Some(old_style);

    if let Some(rect) = *embed.game_rect.lock().unwrap() {
        let _ = set_child_rect(hwnd, rect).map_err(|_| "调整窗口尺寸失败".to_string())?;
    }

    let app_clone = app.clone();
    let embed_state = embed.clone();
    let manager_clone = manager.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            if let Some(handle) = embed_state.projector_handle.lock().unwrap().clone() {
                if !manager_clone.is_running(&handle) {
                    let _ = app_clone.emit("login_status", StatusPayload { status: "Login" });
                    break;
                }
            } else {
                break;
            }
        }
    });

    Ok(())
}

fn resolve_projector_path(app: &AppHandle, cfg: &CoreConfig) -> Result<PathBuf, String> {
    if let Some(path) = cfg.launcher.projector_path.clone() {
        return Ok(path);
    }

    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|_| "无法定位资源目录".to_string())?;
    let path = resource_dir.join("projector.exe");
    if path.exists() {
        Ok(path)
    } else {
        Err("projector.exe 未找到，请在资源目录中提供".to_string())
    }
}

fn redact_url(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        format!("{}{}", parsed.origin().ascii_serialization(), parsed.path())
    } else {
        "redacted".to_string()
    }
}

fn main() {
    rocoknight_core::logging::init_logging();

    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(CoreConfig::default())))
        .manage(ProcessManager::new())
        .manage(Arc::new(EmbedState::default()))
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            is_running,
            set_login_rect,
            set_game_rect,
            start_login_flow,
            stop_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
