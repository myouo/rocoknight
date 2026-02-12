use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info_span, Span};

/// 全局 request_id 计数器（简单递增，避免 UUID 开销）
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 生成新的 request_id
pub fn generate_request_id() -> u64 {
    REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// 为 Tauri command 创建带 request_id 的 span
///
/// # 用法
///
/// ```rust
/// #[tauri::command]
/// fn my_command(app: AppHandle) -> Result<(), String> {
///     let _span = create_command_span("my_command");
///     // 后续所有日志都会自动包含 request_id
///     tracing::info!("doing something");
///     Ok(())
/// }
/// ```
pub fn create_command_span(command_name: &str) -> Span {
    let request_id = generate_request_id();
    info_span!(
        "command",
        cmd = command_name,
        request_id = request_id,
        duration_ms = tracing::field::Empty,
        status = tracing::field::Empty,
    )
}

/// 为阶段操作创建 span
///
/// # 用法
///
/// ```rust
/// let _span = create_stage_span("config_load", "start");
/// // 执行操作
/// tracing::info!("config loaded");
/// ```
pub fn create_stage_span(stage: &str, status: &str) -> Span {
    info_span!(
        "stage",
        stage = stage,
        status = status,
        duration_ms = tracing::field::Empty,
    )
}

/// Command 执行计时器
///
/// 自动记录 command 执行耗时，并在超过阈值时发出警告
///
/// # 用法
///
/// ```rust
/// #[tauri::command]
/// fn my_command() -> Result<(), String> {
///     let _timer = CommandTimer::new("my_command", 200);
///     // 执行操作
///     Ok(())
/// }
/// ```
pub struct CommandTimer {
    command_name: String,
    start: std::time::Instant,
    threshold_ms: u64,
    span: Span,
}

impl CommandTimer {
    /// 创建新的计时器
    ///
    /// # 参数
    ///
    /// - `command_name`: command 名称
    /// - `threshold_ms`: 耗时阈值（毫秒），超过此值会发出警告
    pub fn new(command_name: &str, threshold_ms: u64) -> Self {
        let span = create_command_span(command_name);
        {
            let _enter = span.enter();
            tracing::info!("command started");
        }

        Self {
            command_name: command_name.to_string(),
            start: std::time::Instant::now(),
            threshold_ms,
            span,
        }
    }

    /// 标记 command 成功完成
    pub fn success(self) {
        // Drop 会自动处理
    }

    /// 标记 command 失败
    pub fn fail(self, error: &str) {
        let _enter = self.span.enter();
        tracing::error!(error = error, "command failed");
    }
}

impl Drop for CommandTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let duration_ms = elapsed.as_millis() as u64;

        let _enter = self.span.enter();

        // 记录耗时到 span
        self.span.record("duration_ms", duration_ms);
        self.span.record("status", "completed");

        if duration_ms > self.threshold_ms {
            tracing::warn!(
                duration_ms = duration_ms,
                threshold_ms = self.threshold_ms,
                "command execution slow"
            );
        } else {
            tracing::info!(duration_ms = duration_ms, "command completed");
        }
    }
}

/// 阶段计时器
///
/// 用于记录阶段操作的耗时
pub struct StageTimer {
    stage: String,
    start: std::time::Instant,
    span: Span,
}

impl StageTimer {
    pub fn new(stage: &str) -> Self {
        let span = create_stage_span(stage, "start");
        {
            let _enter = span.enter();
            tracing::info!("stage started");
        }

        Self {
            stage: stage.to_string(),
            start: std::time::Instant::now(),
            span,
        }
    }

    pub fn success(self) {
        // Drop 会自动处理
    }

    pub fn fail(self, error: &str) {
        let _enter = self.span.enter();
        self.span.record("status", "fail");
        tracing::error!(error = error, "stage failed");
    }
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let duration_ms = elapsed.as_millis() as u64;

        let _enter = self.span.enter();
        self.span.record("duration_ms", duration_ms);
        self.span.record("status", "success");

        tracing::info!(duration_ms = duration_ms, "stage completed");
    }
}
