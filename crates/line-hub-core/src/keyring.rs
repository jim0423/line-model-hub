//! Secure API key storage via the OS keyring.
//!
//! Windows: Credential Manager (`wincred`)
//! macOS:   Keychain
//! Linux:   Secret Service (gnome-keyring / kwallet via DBus)
//!
//! On Windows the native keyring writes through `wincred` which keeps the
//! secret encrypted with the user's DPAPI master key — there is no plaintext
//! on disk and no export through `tasklist`. We keep non-secret provider
//! metadata (base URL, default model, nickname) in the JSON config file
//! alongside the encrypted key; only the secret value crosses into the
//! keyring.
//!
//! Each provider is keyed by its stable id (`openai`, `minimax`, `anthropic`,
//! `ollama`) so uninstall / reinstall does not leave orphaned credentials.

use crate::{HubError, HubResult};
use keyring::Entry;

/// Service name registered with the OS keyring. Used to namespace our
/// entries so other apps reading from the same store cannot collide.
const SERVICE_NAME: &str = "com.tt-openclaw.line-xiaobangshou";

/// Store an API key for a provider in the OS keyring.
///
/// Overwrites any existing entry with the same id.
pub fn set_api_key(provider_id: &str, api_key: &str) -> HubResult<()> {
    let entry = Entry::new(SERVICE_NAME, provider_id)
        .map_err(|e| HubError::Keyring(format!("create entry: {e}")))?;
    entry
        .set_password(api_key)
        .map_err(|e| HubError::Keyring(format!("set_password({provider_id}): {e}")))?;
    Ok(())
}

/// Read an API key for a provider from the OS keyring.
///
/// Returns `Ok(None)` when the keyring has no entry for this provider —
/// this is the expected case for Ollama (no key needed) or for a fresh
/// install where the user has not yet entered a key.
pub fn get_api_key(provider_id: &str) -> HubResult<Option<String>> {
    let entry = Entry::new(SERVICE_NAME, provider_id)
        .map_err(|e| HubError::Keyring(format!("create entry: {e}")))?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(HubError::Keyring(format!(
            "get_password({provider_id}): {e}"
        ))),
    }
}

/// Delete an API key from the keyring (best-effort).
///
/// Returns `Ok(true)` if a credential was actually removed, `Ok(false)` if
/// there was nothing to remove.
pub fn delete_api_key(provider_id: &str) -> HubResult<bool> {
    let entry = Entry::new(SERVICE_NAME, provider_id)
        .map_err(|e| HubError::Keyring(format!("create entry: {e}")))?;
    match entry.delete_credential() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(HubError::Keyring(format!(
            "delete_credential({provider_id}): {e}"
        ))),
    }
}

/// List all provider ids that have a key stored in the keyring under our
/// service name. Used by the Tauri command `list_keyring_providers` so the
/// UI can render a "configured" badge on the Settings dialog. We probe the
/// four canonical providers we ship with; ad-hoc ids (e.g. probe entries
/// left by tests) are not enumerated.
pub fn list_configured_providers() -> Vec<String> {
    let candidates = ["openai", "minimax", "anthropic", "ollama"];
    candidates
        .iter()
        .filter(|id| get_api_key(id).ok().flatten().is_some())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Skip keyring tests on CI that has no Secret Service / Credential
    /// Manager. We detect by checking the user override env var.
    fn headless() -> bool {
        env::var_os("LINE_HUB_SKIP_KEYRING_TESTS").is_some()
    }

    #[test]
    fn set_get_roundtrip() {
        if headless() {
            eprintln!("skipping (LINE_HUB_SKIP_KEYRING_TESTS set)");
            return;
        }
        let id = "test-roundtrip";
        set_api_key(id, "sk-secret-test-value").expect("set");
        let got = get_api_key(id).expect("get").expect("present");
        assert_eq!(got, "sk-secret-test-value");
        let removed = delete_api_key(id).expect("delete");
        assert!(removed);
        let gone = get_api_key(id).expect("get after delete");
        assert!(gone.is_none());
    }

    #[test]
    fn get_missing_returns_none() {
        if headless() {
            return;
        }
        let got = get_api_key("definitely-not-a-real-provider-id").expect("get");
        assert!(got.is_none());
    }

    #[test]
    fn delete_missing_returns_false() {
        if headless() {
            return;
        }
        let removed = delete_api_key("definitely-not-a-real-provider-id-2").expect("delete");
        assert!(!removed);
    }

    #[test]
    fn list_configured_providers_works_after_set_and_delete() {
        if headless() {
            return;
        }
        // Use one of the canonical provider ids so the list probe picks it up.
        let probe_id = "minimax";
        let prior = get_api_key(probe_id).expect("get prior");
        set_api_key(probe_id, "list-probe").expect("set probe");
        let ids = list_configured_providers();
        assert!(
            ids.iter().any(|id| id == probe_id),
            "list did not contain probe id: {ids:?}"
        );
        // Restore the prior state (delete the probe if nothing was there).
        match prior {
            Some(original) => set_api_key(probe_id, &original).expect("restore"),
            None => {
                let _ = delete_api_key(probe_id);
            }
        }
    }
}
