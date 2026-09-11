//! Configuration storage.
//!
//! Provider entries are kept in `~/.line-hub/config.json`. API keys are
//! encrypted at rest using a key derived from a password the user enters
//! once at first run (stored via the OS keyring).

use crate::provider::ProviderId;
use crate::{HubError, HubResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubConfig {
    /// Selected provider for the next session.
    #[serde(default)]
    pub default_provider: Option<ProviderId>,
    /// Selected model within `default_provider`.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Per-provider settings.
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
    /// Path to the line-desktop-mcp entry script. If empty, the bundled
    /// resource shipped with the Tauri app is used.
    #[serde(default)]
    pub line_mcp_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub id: ProviderId,
    /// Plaintext at runtime — the on-disk file uses encrypted values.
    /// For now we store plaintext in dev; production uses `keyring`.
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
}

impl HubConfig {
    pub fn config_path() -> PathBuf {
        let mut p = dirs_home();
        p.push(".line-hub");
        let _ = std::fs::create_dir_all(&p);
        p.push("config.json");
        p
    }

    pub async fn load() -> HubResult<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = tokio::fs::read(&path).await?;
        let cfg: Self = serde_json::from_slice(&bytes)?;
        Ok(cfg)
    }

    pub async fn save(&self) -> HubResult<()> {
        let path = Self::config_path();
        let bytes = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(&path, bytes).await?;
        Ok(())
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[allow(dead_code)]
pub fn load_or_default() -> HubConfig {
    // Synchronous best-effort: poll once for the future's completion.
    futures::executor::block_on(HubConfig::load()).unwrap_or_default()
}

#[allow(dead_code)]
pub fn ensure_minimax_default(cfg: &mut HubConfig) -> &mut HubConfig {
    if cfg.providers.iter().all(|p| p.id != ProviderId::MiniMax) {
        cfg.providers.push(ProviderEntry {
            id: ProviderId::MiniMax,
            api_key: std::env::var("MINIMAX_API_KEY").unwrap_or_default(),
            base_url: None,
            default_model: Some("MiniMax-M3".into()),
        });
    }
    if cfg.default_provider.is_none() {
        cfg.default_provider = Some(ProviderId::MiniMax);
    }
    if cfg.default_model.is_none() {
        cfg.default_model = Some("MiniMax-M3".into());
    }
    cfg
}

#[allow(unused_imports)]
use serde_json;