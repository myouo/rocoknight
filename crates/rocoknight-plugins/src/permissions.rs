use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionSet {
    pub config_read: bool,
    pub config_write: bool,
    pub process_control: bool,
    pub window_control: bool,
    pub notifications: bool,
    pub network: NetworkPermission,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkPermission {
    pub allowed_domains: Vec<String>,
}

