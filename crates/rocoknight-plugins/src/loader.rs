use crate::manifest::PluginManifest;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub root_dir: PathBuf,
}

pub struct PluginLoader {
    root: PathBuf,
}

impl PluginLoader {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn discover(&self) -> anyhow::Result<Vec<LoadedPlugin>> {
        let mut items = Vec::new();
        if !self.root.exists() {
            return Ok(items);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(manifest) = Self::read_manifest(&path)? {
                    items.push(LoadedPlugin { manifest, root_dir: path });
                }
            }
        }
        Ok(items)
    }

    fn read_manifest(dir: &Path) -> anyhow::Result<Option<PluginManifest>> {
        let manifest_path = dir.join("plugin.json");
        if !manifest_path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(manifest_path)?;
        let manifest: PluginManifest = serde_json::from_str(&data)?;
        Ok(Some(manifest))
    }
}

