use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::debug_log_bus::{LogEvent, push_log};

/// Tracing Layer，将日志事件推送到 Debug Console
pub struct DebugConsoleLayer;

impl DebugConsoleLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for DebugConsoleLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // 提取日志级别
        let level = event.metadata().level().as_str();

        // 提取目标（模块路径）
        let target = event.metadata().target();

        // 提取消息和字段
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        // 提取 span 上下文（span 名）
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                // 当前仅附加 span 名称，字段提取后续可按需扩展
                let span_name = span.name();
                visitor
                    .fields
                    .insert("span".to_string(), span_name.to_string());
            }
        }

        // 构建消息（包含结构化字段）
        let mut message = visitor.message.clone();

        // 添加 visitor 收集的字段
        for (key, value) in visitor.fields.iter() {
            if key != "message" {
                message.push_str(&format!(" {}={}", key, value));
            }
        }

        let mut log_event = LogEvent::new(level, target, message);

        // 添加结构化字段（JSON 格式）
        if !visitor.fields.is_empty() {
            if let Ok(json) = serde_json::to_string(&visitor.fields) {
                log_event.fields = Some(json);
            }
        }

        // 推送到日志总线
        push_log(log_event);
    }
}

/// 访问者，用于提取日志消息和字段
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: std::collections::HashMap<String, String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let field_name = field.name();
        let field_value = format!("{:?}", value);

        if field_name == "message" {
            self.message = field_value;
            // 移除首尾的引号
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        } else {
            self.fields.insert(field_name.to_string(), field_value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let field_name = field.name();

        if field_name == "message" {
            self.message = value.to_string();
        } else {
            self.fields.insert(field_name.to_string(), value.to_string());
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), value.to_string());
    }
}

