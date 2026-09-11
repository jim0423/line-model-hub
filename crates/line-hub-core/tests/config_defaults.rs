//! Unit tests for the hub-level configuration loader.

use line_hub_core::config::{ensure_minimax_default, HubConfig};
use line_hub_core::provider::ProviderId;

/// Calling `ensure_minimax_default` on a fully empty config must produce a
/// config that exposes MiniMax as the default provider. This guards the
/// invariant the Settings dialog depends on — see `SettingsDialog` in
/// `src/App.tsx`.
#[test]
fn ensure_minimax_default_adds_minimax_to_empty_config() {
    let mut cfg = HubConfig::default();
    assert!(
        cfg.providers.is_empty(),
        "freshly-derived Default::default() must be empty so this test is meaningful"
    );

    let cfg = ensure_minimax_default(&mut cfg);

    assert_eq!(cfg.providers.len(), 1, "expected exactly one provider entry");
    assert_eq!(cfg.providers[0].id, ProviderId::MiniMax);
    assert_eq!(
        cfg.default_provider,
        Some(ProviderId::MiniMax),
        "MiniMax should be the active provider by default"
    );
    assert_eq!(cfg.default_model.as_deref(), Some("MiniMax-M3"));
}

/// `ensure_minimax_default` must be idempotent: calling it twice must not add
/// a second MiniMax entry.
#[test]
fn ensure_minimax_default_is_idempotent() {
    let mut cfg = HubConfig::default();
    ensure_minimax_default(&mut cfg);
    let first_count = cfg.providers.len();

    ensure_minimax_default(&mut cfg);
    assert_eq!(
        cfg.providers.len(),
        first_count,
        "ensure_minimax_default must not duplicate entries"
    );
}