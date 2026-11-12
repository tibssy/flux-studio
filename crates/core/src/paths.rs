use std::path::PathBuf;

use crate::identity::{APP_DIR, DEFAULT_OUTPUT_FOLDER};

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
}

pub fn history_db() -> PathBuf {
    data_dir().join("history.db")
}

pub fn thumbs_dir() -> PathBuf {
    cache_dir().join("thumbs")
}

pub fn default_models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn default_output_dir() -> PathBuf {
    dirs::picture_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DEFAULT_OUTPUT_FOLDER)
}
