use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEvent {
    pub topic: String,
    pub payload: serde_json::Value,
}

pub trait EventBus: Send + Sync {
    fn emit(&self, event: BusEvent);
    fn subscribe(&self, topic: &str);
}

