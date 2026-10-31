//! TOML config-file loading.

use daedra::cache::CacheConfig;
use std::time::Duration;

/// Defaults loaded from a TOML file (--config or
/// ~/.config/daedra/daedra.toml). Flags always win over file values.
#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct FileConfig {
    pub(crate) num_results: Option<usize>,
    pub(crate) region: Option<String>,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) cache_ttl_secs: Option<u64>,
}

pub(super) fn load_file_config(path: Option<&str>) -> FileConfig {
    let resolved = path.map(std::path::PathBuf::from).or_else(|| {
        let home = std::env::var("HOME").ok()?;
        let cfg = std::path::PathBuf::from(home).join(".config/daedra/daedra.toml");
        cfg.is_file().then_some(cfg)
    });
    let Some(p) = resolved else {
        return FileConfig::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("warning: could not parse {}: {e}", p.display());
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

pub(super) fn build_cache_config(no_cache: bool, cache_ttl: u64) -> CacheConfig {
    if no_cache {
        CacheConfig {
            enabled: false,
            ..Default::default()
        }
    } else {
        CacheConfig {
            ttl: Duration::from_secs(cache_ttl),
            enabled: true,
            ..Default::default()
        }
    }
}
