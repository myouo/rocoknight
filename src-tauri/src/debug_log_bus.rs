use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tracing::Level;

// ============================================================================
// 配置常量
// ============================================================================

/// 批量发送间隔（毫秒）
const BATCH_INTERVAL_MS: u64 = 200;

/// 内存中保留的历史日志数量（用于窗口打开时回放）
const RING_BUFFER_SIZE: usize = 500;

/// 队列最大容量（超过后丢弃低优先级日志）
const MAX_QUEUE_SIZE: usize = 2000;

/// 单次批量发送的最大日志数
const MAX_BATCH_SIZE: usize = 100;

// ============================================================================
// 数据结构
// ============================================================================

#[derive(Clone, serde::Serialize, Debug)]
pub struct LogEvent {
    /// Unix 时间戳（毫秒）
    pub timestamp: u64,
    /// 日志级别（ERROR, WARN, INFO, DEBUG, TRACE）
    pub level: String,
    /// 日志来源（模块路径）
    pub target: String,
    /// 日志消息
    pub message: String,
    /// 线程 ID（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// 结构化字段（JSON 字符串，可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<String>,
}

impl LogEvent {
    pub fn new(level: &str, target: &str, message: String) -> Self {
        Self {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            level: level.to_uppercase(),
            target: target.to_string(),
            message,
            thread_id: None,
            fields: None,
        }
    }

    pub fn priority(&self) -> u8 {
        match self.level.as_str() {
            "ERROR" => 5,
            "WARN" => 4,
            "INFO" => 3,
            "DEBUG" => 2,
            "TRACE" => 1,
            _ => 0,
        }
    }
}

// ============================================================================
// 全局状态
// ============================================================================

struct LogBusState {
    /// 待发送队列
    queue: VecDeque<LogEvent>,
    /// 历史日志环形缓冲区（用于回放）
    ring_buffer: VecDeque<LogEvent>,
    /// Debug 窗口是否打开
    window_open: bool,
    /// 丢弃统计
    dropped_count: usize,
    /// 统计信息
    stats: LogBusStats,
}

/// 日志总线统计信息
#[derive(Clone, serde::Serialize, Debug)]
pub struct LogBusStats {
    /// 总接收日志数
    pub total_received: usize,
    /// 总发送日志数
    pub total_sent: usize,
    /// 总丢弃日志数
    pub total_dropped: usize,
    /// 当前队列长度
    pub queue_length: usize,
    /// 当前环形缓冲区长度
    pub ring_buffer_length: usize,
    /// 最近 1 秒的日志速率（条/秒）
    pub log_rate_per_sec: f64,
    /// 最后更新时间
    pub last_update_time: u64,
}

impl Default for LogBusStats {
    fn default() -> Self {
        Self {
            total_received: 0,
            total_sent: 0,
            total_dropped: 0,
            queue_length: 0,
            ring_buffer_length: 0,
            log_rate_per_sec: 0.0,
            last_update_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }
}

impl LogBusState {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            ring_buffer: VecDeque::new(),
            window_open: false,
            dropped_count: 0,
            stats: LogBusStats::default(),
        }
    }

    fn update_stats(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let elapsed_ms = now - self.stats.last_update_time;

        if elapsed_ms > 0 {
            // 计算日志速率（条/秒）
            let received_since_last = self.stats.total_received;
            self.stats.log_rate_per_sec = (received_since_last as f64 * 1000.0) / elapsed_ms as f64;
        }

        self.stats.queue_length = self.queue.len();
        self.stats.ring_buffer_length = self.ring_buffer.len();
        self.stats.total_dropped = self.dropped_count;
        self.stats.last_update_time = now;
    }
}

static LOG_BUS: OnceLock<Arc<Mutex<LogBusState>>> = OnceLock::new();
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static FLUSH_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
static SHOULD_EXIT: AtomicBool = AtomicBool::new(false); // 新增：退出标志

// ============================================================================
// 公共 API
// ============================================================================

/// 初始化日志总线（在 Tauri setup 中调用）
pub fn init(app_handle: AppHandle) {
    let _ = APP_HANDLE.set(app_handle);
    let _ = LOG_BUS.set(Arc::new(Mutex::new(LogBusState::new())));

    // 启动后台刷新线程
    if !FLUSH_THREAD_RUNNING.swap(true, Ordering::SeqCst) {
        std::thread::spawn(flush_loop);
    }

    tracing::info!("[LogBus] Initialized");
}

/// 推送日志事件到总线
pub fn push_log(event: LogEvent) {
    // 如果正在退出，立即返回，不做任何操作
    if crate::EXITING.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let Some(bus) = LOG_BUS.get() else {
        return;
    };

    // 锁诊断：尝试获取锁
    crate::request_context::cmd_log("BUS_LOCK_TRY push_log");

    // 使用 try_lock 避免阻塞，如果锁忙就丢弃日志
    let mut state = match bus.try_lock() {
        Ok(guard) => {
            crate::request_context::cmd_log("BUS_LOCK_OK push_log");
            guard
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            // 锁被占用，丢弃日志，不阻塞调用者
            crate::request_context::cmd_log("BUS_LOCK_BUSY push_log (dropping log)");
            return;
        }
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            crate::request_context::cmd_log("BUS_LOCK_POISONED push_log (recovering)");
            poisoned.into_inner()
        }
    };

    // 更新统计
    state.stats.total_received += 1;

    // 更新环形缓冲区（始终保留最近的日志）
    state.ring_buffer.push_back(event.clone());
    if state.ring_buffer.len() > RING_BUFFER_SIZE {
        state.ring_buffer.pop_front();
    }

    // 如果窗口未打开，不推送到队列
    if !state.window_open {
        return;
    }

    // 检查队列是否已满
    if state.queue.len() >= MAX_QUEUE_SIZE {
        // 丢弃低优先级日志（DEBUG/TRACE）
        if event.priority() <= 2 {
            state.dropped_count += 1;
            return;
        }

        // 如果是高优先级日志，尝试丢弃队列中的低优先级日志
        if let Some(pos) = state.queue.iter().position(|e| e.priority() <= 2) {
            state.queue.remove(pos);
            state.dropped_count += 1;
        } else {
            // 队列全是高优先级日志，丢弃当前日志
            state.dropped_count += 1;
            return;
        }
    }

    state.queue.push_back(event);
}

/// 设置 Debug 窗口状态
pub fn set_window_open(open: bool) {
    // 如果正在退出，立即返回，不做任何操作
    if crate::EXITING.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    // 诊断日志：记录窗口状态变化
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let path = std::path::PathBuf::from(local)
                .join("RocoKnight")
                .join("logs")
                .join("rocoknight.log");
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(file, "[{:?}] LOG_BUS: set_window_open({})", std::time::SystemTime::now(), open);
            }
        }
    }

    let Some(bus) = LOG_BUS.get() else {
        return;
    };

    let mut state = match bus.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("[LogBus] Mutex poisoned in set_window_open, recovering...");
            poisoned.into_inner()
        }
    };
    let was_open = state.window_open;
    state.window_open = open;

    // 窗口从关闭到打开：不发送历史日志（防止退出时阻塞）
    // 注释掉历史日志发送，避免在退出时触发 emit_batch
    if !was_open && open {
        // 诊断日志
        #[cfg(target_os = "windows")]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let path = std::path::PathBuf::from(local)
                    .join("RocoKnight")
                    .join("logs")
                    .join("rocoknight.log");
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    let _ = writeln!(file, "[{:?}] LOG_BUS: skipping history logs (size: {})", std::time::SystemTime::now(), state.ring_buffer.len());
                }
            }
        }

        // 不发送历史日志，避免在退出时触发 emit_batch
        // let history: Vec<LogEvent> = state.ring_buffer.iter().cloned().collect();
        // drop(state);
        // if !history.is_empty() {
        //     emit_batch(history);
        // }
    }

    tracing::info!("[LogBus] Window state changed: open={}", open);
}
/// 获取当前窗口状态
pub fn is_window_open() -> bool {
    LOG_BUS
        .get()
        .and_then(|bus| bus.lock().ok().map(|state| state.window_open))
        .unwrap_or(false)
}

/// 获取日志总线统计信息
pub fn get_stats() -> LogBusStats {
    LOG_BUS
        .get()
        .and_then(|bus| {
            bus.lock().ok().map(|mut state| {
                state.update_stats();
                state.stats.clone()
            })
        })
        .unwrap_or_default()
}

/// 获取最近的 N 条历史日志（用于 debug 窗口初次打开）
pub fn get_recent_logs(limit: usize) -> Vec<LogEvent> {
    LOG_BUS
        .get()
        .and_then(|bus| {
            bus.lock().ok().map(|state| {
                let count = state.ring_buffer.len().min(limit);
                state
                    .ring_buffer
                    .iter()
                    .rev() // 最新的在前
                    .take(count)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev() // 恢复时间顺序
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// 停止日志总线（在程序退出时调用）
pub fn shutdown() {
    tracing::info!("[LogBus] Shutting down...");
    // 设置退出标志，让 flush_loop 自然退出
    SHOULD_EXIT.store(true, Ordering::Relaxed);
    // 不等待线程退出，因为我们准备强制退出进程
    tracing::info!("[LogBus] Exit flag set, flush loop will exit on next check");
}

/// 获取退出标志（用于 watchdog 诊断）
pub fn is_should_exit() -> bool {
    SHOULD_EXIT.load(Ordering::Relaxed)
}

// ============================================================================
// 内部实现
// ============================================================================

/// 后台刷新循环
fn flush_loop() {
    tracing::info!("[LogBus] Flush thread started");

    loop {
        // 检查是否应该退出（每次循环都检查）
        if SHOULD_EXIT.load(Ordering::Relaxed) || crate::EXITING.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("[LogBus] Flush thread exiting");
            FLUSH_THREAD_RUNNING.store(false, Ordering::SeqCst);
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(BATCH_INTERVAL_MS));

        // sleep 后立即检查退出标志
        if SHOULD_EXIT.load(Ordering::Relaxed) || crate::EXITING.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("[LogBus] Flush thread exiting after sleep");
            FLUSH_THREAD_RUNNING.store(false, Ordering::SeqCst);
            break;
        }

        let Some(bus) = LOG_BUS.get() else {
            continue;
        };

        let (batch, stats): (Vec<LogEvent>, LogBusStats) = {
            let mut state = match bus.lock() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    eprintln!("[LogBus] Mutex poisoned in flush_loop, recovering...");
                    poisoned.into_inner()
                }
            };

            if state.queue.is_empty() {
                continue;
            }

            // 取出一批日志
            let count = state.queue.len().min(MAX_BATCH_SIZE);
            let batch: Vec<LogEvent> = state.queue.drain(..count).collect();

            // 更新统计
            state.stats.total_sent += batch.len();
            state.update_stats();

            (batch, state.stats.clone())
        };

        if !batch.is_empty() {
            emit_batch(batch);
            // 同时发送统计信息
            emit_stats(stats);
        }
    }
}

/// 向前端发送批量日志（带超时保护）
fn emit_batch(batch: Vec<LogEvent>) {
    // BUS_EMIT_ENTER
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let path = std::path::PathBuf::from(local)
                .join("RocoKnight")
                .join("logs")
                .join("rocoknight.log");
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(file, "[{:?}] BUS_EMIT_ENTER: batch size={}", std::time::SystemTime::now(), batch.len());
            }
        }
    }

    let Some(app) = APP_HANDLE.get() else {
        return;
    };

    // 检查是否正在退出（必须第一个检查，防止任何 emit 操作）
    if SHOULD_EXIT.load(Ordering::Relaxed) {
        #[cfg(target_os = "windows")]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let path = std::path::PathBuf::from(local)
                    .join("RocoKnight")
                    .join("logs")
                    .join("rocoknight.log");
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    let _ = writeln!(file, "[{:?}] BUS_EMIT_SKIP: SHOULD_EXIT=true", std::time::SystemTime::now());
                }
            }
        }
        return;
    }

    // 检查窗口是否打开（只在窗口打开时发送）
    if !is_window_open() {
        #[cfg(target_os = "windows")]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let path = std::path::PathBuf::from(local)
                    .join("RocoKnight")
                    .join("logs")
                    .join("rocoknight.log");
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    let _ = writeln!(file, "[{:?}] BUS_EMIT_SKIP: window_open=false", std::time::SystemTime::now());
                }
            }
        }
        return;
    }

    // 检查窗口是否存在
    match app.get_webview_window("debug") {
        Some(_) => {
            // 窗口存在，继续
        }
        None => {
            #[cfg(target_os = "windows")]
            {
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    let path = std::path::PathBuf::from(local)
                        .join("RocoKnight")
                        .join("logs")
                        .join("rocoknight.log");
                    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(file, "[{:?}] BUS_EMIT_ERR: debug window not found", std::time::SystemTime::now());
                    }
                }
            }
            return;
        }
    }

    // 使用线程 + 超时机制，避免 emit 阻塞
    let (tx, rx) = std::sync::mpsc::channel();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        let result = app_clone.emit("debug_log_batch", &batch);
        let _ = tx.send(result);
    });

    // 等待最多 100ms
    match rx.recv_timeout(std::time::Duration::from_millis(100)) {
        Ok(Ok(())) => {
            // 发送成功
            #[cfg(target_os = "windows")]
            {
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    let path = std::path::PathBuf::from(local)
                        .join("RocoKnight")
                        .join("logs")
                        .join("rocoknight.log");
                    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(file, "[{:?}] BUS_EMIT_OK", std::time::SystemTime::now());
                    }
                }
            }
        }
        Ok(Err(e)) => {
            eprintln!("[LogBus] Failed to emit batch: {}", e);
            #[cfg(target_os = "windows")]
            {
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    let path = std::path::PathBuf::from(local)
                        .join("RocoKnight")
                        .join("logs")
                        .join("rocoknight.log");
                    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(file, "[{:?}] BUS_EMIT_ERR: {:?}", std::time::SystemTime::now(), e);
                    }
                }
            }
        }
        Err(_) => {
            eprintln!("[LogBus] Emit batch timeout");
            #[cfg(target_os = "windows")]
            {
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    let path = std::path::PathBuf::from(local)
                        .join("RocoKnight")
                        .join("logs")
                        .join("rocoknight.log");
                    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(file, "[{:?}] BUS_EMIT_TIMEOUT", std::time::SystemTime::now());
                    }
                }
            }
        }
    }
}
/// 向前端发送统计信息（带超时保护）
fn emit_stats(stats: LogBusStats) {
    let Some(app) = APP_HANDLE.get() else {
        return;
    };

    // 检查是否正在退出（必须第一个检查，防止任何 emit 操作）
    if SHOULD_EXIT.load(Ordering::Relaxed) {
        return;
    }

    // 检查窗口是否打开（只在窗口打开时发送）
    if !is_window_open() {
        return;
    }

    // 使用线程 + 超时机制，避免 emit 阻塞
    let (tx, rx) = std::sync::mpsc::channel();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        let result = app_clone.emit("debug_log_stats", &stats);
        let _ = tx.send(result);
    });

    // 等待最多 50ms
    match rx.recv_timeout(std::time::Duration::from_millis(50)) {
        Ok(Ok(())) => {
            // 发送成功
        }
        Ok(Err(e)) => {
            eprintln!("[LogBus] Failed to emit stats: {}", e);
        }
        Err(_) => {
            eprintln!("[LogBus] Emit stats timeout");
        }
    }
}

// ============================================================================
// 便捷宏（用于快速记录日志）
// ============================================================================

#[macro_export]
macro_rules! bus_log {
    ($level:expr, $target:expr, $($arg:tt)*) => {
        {
            let msg = format!($($arg)*);
            let event = $crate::debug_log_bus::LogEvent::new($level, $target, msg);
            $crate::debug_log_bus::push_log(event);
        }
    };
}

#[macro_export]
macro_rules! bus_error {
    ($($arg:tt)*) => {
        $crate::bus_log!("ERROR", module_path!(), $($arg)*)
    };
}

#[macro_export]
macro_rules! bus_warn {
    ($($arg:tt)*) => {
        $crate::bus_log!("WARN", module_path!(), $($arg)*)
    };
}

#[macro_export]
macro_rules! bus_info {
    ($($arg:tt)*) => {
        $crate::bus_log!("INFO", module_path!(), $($arg)*)
    };
}

#[macro_export]
macro_rules! bus_debug {
    ($($arg:tt)*) => {
        $crate::bus_log!("DEBUG", module_path!(), $($arg)*)
    };
}

// ============================================================================
// dbglog! 宏（统一接口，推荐使用）
// ============================================================================

/// 统一的 debug 日志宏（推荐使用）
/// 用法：dbglog!(INFO, "message"); dbglog!(ERROR, "error: {}", err);
#[macro_export]
macro_rules! dbglog {
    (TRACE, $($arg:tt)*) => {
        $crate::bus_log!("TRACE", module_path!(), $($arg)*)
    };
    (DEBUG, $($arg:tt)*) => {
        $crate::bus_log!("DEBUG", module_path!(), $($arg)*)
    };
    (INFO, $($arg:tt)*) => {
        $crate::bus_log!("INFO", module_path!(), $($arg)*)
    };
    (WARN, $($arg:tt)*) => {
        $crate::bus_log!("WARN", module_path!(), $($arg)*)
    };
    (ERROR, $($arg:tt)*) => {
        $crate::bus_log!("ERROR", module_path!(), $($arg)*)
    };
}
