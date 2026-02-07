use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoreConfig {
    pub launcher: LauncherConfig,
    pub game: GameConfig,
    pub cache: CacheConfig,
    pub update: UpdateConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LauncherConfig {
    pub projector_path: Option<PathBuf>,
    pub main_swf_url: Option<String>,
    pub allow_multi_instance: bool,
    pub auto_restart_on_crash: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameConfig {
    pub window_title: Option<String>,
    pub fps_limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheConfig {
    pub root_dir: Option<PathBuf>,
    pub max_size_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateConfig {
    pub channel: Option<String>,
    pub allow_prerelease: bool,
}

