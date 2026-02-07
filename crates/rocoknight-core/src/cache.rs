use crate::error::{CoreError, CoreResult};
use std::path::PathBuf;

pub struct CacheManager {
    root: PathBuf,
}

impl CacheManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root_dir(&self) -> &PathBuf {
        &self.root
    }

    pub fn purge(&self) -> CoreResult<()> {
        Err(CoreError::Config("cache purge not implemented".to_string()))
    }
}

