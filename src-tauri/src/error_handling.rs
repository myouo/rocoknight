use std::error::Error;

/// 格式化错误链，包含所有 source
///
/// # 用法
///
/// ```rust
/// match some_operation() {
///     Err(e) => {
///         let error_chain = format_error_chain(&e);
///         tracing::error!(error = %error_chain, "operation failed");
///     }
/// }
/// ```
pub fn format_error_chain<E: Error>(error: &E) -> String {
    let mut chain = vec![error.to_string()];
    let mut source = error.source();

    while let Some(err) = source {
        chain.push(format!("  caused by: {}", err));
        source = err.source();
    }

    chain.join("\n")
}

/// 记录错误到 tracing，包含完整错误链
///
/// # 用法
///
/// ```rust
/// if let Err(e) = some_operation() {
///     log_error("operation failed", &e);
/// }
/// ```
pub fn log_error<E: Error>(context: &str, error: &E) {
    let error_chain = format_error_chain(error);
    tracing::error!(
        context = context,
        error = %error_chain,
        "error occurred"
    );
}

/// 记录错误到 tracing（带 request_id）
pub fn log_error_with_context<E: Error>(context: &str, error: &E, request_id: u64) {
    let error_chain = format_error_chain(error);
    tracing::error!(
        context = context,
        error = %error_chain,
        request_id = request_id,
        "error occurred"
    );
}

/// 将 Result 转换为带错误链的 String
///
/// 用于 Tauri command 返回值
pub fn result_to_string<T, E: Error>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|e| format_error_chain(&e))
}

/// 错误结构体，用于 Debug Console 显示
#[derive(serde::Serialize, Clone)]
pub struct ErrorInfo {
    /// 错误消息
    pub message: String,
    /// 错误链
    pub chain: Vec<String>,
    /// 上下文信息
    pub context: Option<String>,
    /// request_id
    pub request_id: Option<u64>,
}

impl ErrorInfo {
    pub fn from_error<E: Error>(error: &E) -> Self {
        let mut chain = vec![error.to_string()];
        let mut source = error.source();

        while let Some(err) = source {
            chain.push(err.to_string());
            source = err.source();
        }

        Self {
            message: error.to_string(),
            chain,
            context: None,
            request_id: None,
        }
    }

    pub fn with_context(mut self, context: String) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_request_id(mut self, request_id: u64) -> Self {
        self.request_id = Some(request_id);
        self
    }
}
