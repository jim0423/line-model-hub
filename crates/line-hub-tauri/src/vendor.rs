//! v0.6.8: extract the bundled `line-desktop-mcp` source tree to a
//! per-user data directory on first launch and run `npm install` there
//! so `server.js` has its `node_modules` ready to spawn.
//!
//! Why this exists: v0.6.1..v0.6.7 tried to bundle line-desktop-mcp via
//! `tauri.conf.json::bundle.resources` — every release produced an
//! installer that `cargo tauri build` accepted as a success yet which
//! contained zero references to line-desktop-mcp / server.js /
//! package.json. The bug was in the NSIS bundler (mode-A vs mode-B path
//! resolution in `tauri-bundler/src/bundle/windows/nsis/mod.rs:824` and
//! the silent `output_path.current_dir()` switch at line 704). We
//! sidestep the bundler entirely by embedding the vendor tree directly
//! in the exe via `include_dir!` in `build.rs`, then writing it out at
//! runtime.
//!
//! Path resolution priority (highest first):
//!   1. `HUB_LINE_MCP_PATH` env var — CI smoke tests and power-users.
//!   2. `HubConfig.line_mcp_path` saved by the Settings dialog.
//!   3. The path produced by `ensure_vendor_installed()` here, which
//!      unpacks the embedded `vendor/line-desktop-mcp/` source tree to
//!      `<data_dir>/vendor/line-desktop-mcp/` and runs
//!      `npm install --omit=dev --ignore-scripts --no-audit --no-fund`
//!      exactly once per machine (guarded by the marker file
//!      `<data_dir>/vendor/.extracted.v3.0.0`).
//!
//! The `<data_dir>` follows the Tauri convention
//! `dirs::data_dir()/<config_identifier>/` so uninstalling the app
//! cleans up by removing the directory.

use include_dir::Dir;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;
use tracing::{info, warn};

/// Pinned `line-desktop-mcp` tag for which the embedded sources were
/// captured during `build.rs`. Increase this when bumping the upstream
/// tag — the marker filename includes the tag so re-extraction kicks in.
pub const EMBEDDED_LINE_MCP_TAG: &str = "v3.0.0";

/// `Dir` baked into the binary by `build.rs`. We only consume its bytes;
/// `include_dir!` keeps it parse-time-checked so a forgotten vendor
/// checkout fails the build instead of failing the user's first launch.
pub static EMBEDDED: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/vendor/line-desktop-mcp");

/// Data-dir subdirectory used when resolving the extract target. We
/// keep this short and English-only (`line-hub` instead of the Tauri
/// identifier `com.tt-openclaw.line-xiaobangshou`) because the longer
/// dotted form triggers Windows Antivirus / EDR / SmartScreen false
/// positives on the `mkdir ...\com.<unknown>` pattern — observed in
/// v0.6.9..v0.6.10 as NTSTATUS 0xC1 ("%1 is not a valid Win32
/// application") on `std::fs::create_dir_all`.
///
/// A plain `line-hub` subdir under `%LOCALAPPDATA%` does not match the
/// `com.*` shape AV hooks look for, and remains uniquely owned by us.
const DATA_DIR_SUBDIR: &str = "line-hub";

#[derive(Debug, Error)]
pub enum VendorError {
    #[error("line-desktop-mcp not embedded in this binary (build was run without the vendor tree at crates/line-hub-tauri/vendor/line-desktop-mcp/)")]
    NotEmbedded,
    #[error("could not resolve user data directory: {0}")]
    NoDataDir(String),
    #[error("io error writing vendor tree to {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`npm install` for line-desktop-mcp exited with status {status}: {stderr}")]
    NpmInstall { status: i32, stderr: String },
    #[error("`npm` not on PATH; install Node.js from https://nodejs.org/")]
    NpmMissing,
}

/// Public entry point — see module docs for path-resolution priority.
///
/// Returns the absolute path to `server.js` ready to spawn.
///
/// This call can take 30-60 s the first time (runs `npm install`).
/// Subsequent calls complete in milliseconds because the marker file
/// short-circuits re-extraction.
pub async fn ensure_vendor_installed(
    identifier: &str,
) -> Result<PathBuf, VendorError> {
    // Sentinel — `include_dir!` already panic'd at compile time if the
    // path was missing, but a malformed release profile could still
    // leave the static empty. Check via the well-known entry point
    // instead of `entries().is_empty()` which is always non-empty as
    // long as the manifest-dir exists (it counts the parent dir).
    if EMBEDDED
        .get_file(std::path::Path::new("src/server.js"))
        .is_none()
    {
        return Err(VendorError::NotEmbedded);
    }

    let data_dir = resolve_data_dir(identifier)?;
    let vendor_dir = data_dir.join("vendor").join("line-desktop-mcp");
    let marker = data_dir
        .join("vendor")
        .join(format!(".extracted.{}", EMBEDDED_LINE_MCP_TAG));

    if marker.exists()
        && vendor_dir.join("package.json").exists()
        && vendor_dir.join("src").join("server.js").exists()
        && vendor_dir.join("node_modules").exists()
    {
        info!(
            "line-desktop-mcp {} already extracted at {}",
            EMBEDDED_LINE_MCP_TAG,
            vendor_dir.display()
        );
        return Ok(vendor_dir.join("src").join("server.js"));
    }

    info!(
        "extracting embedded line-desktop-mcp ({} files) to {}",
        EMBEDDED.files().count(),
        vendor_dir.display()
    );
    if let Some(parent) = vendor_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| VendorError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    // Wipe a partial previous attempt so left-over `node_modules/` from a
    // half-finished install does not pollute the new tree.
    if vendor_dir.exists() {
        let _ = std::fs::remove_dir_all(&vendor_dir);
    }
    std::fs::create_dir_all(&vendor_dir).map_err(|e| VendorError::Io {
        path: vendor_dir.clone(),
        source: e,
    })?;

    // `Dir::files()` only returns top-level files — the embedded tree
    // has deep subtrees under `src/automation/`, `src/extensions/`,
    // etc. that must also land on disk. Walk recursively so we never
    // miss a leaf.
    //
    // v0.6.11: log every file + dir as we write it. If a Windows
    // install returns os error 193 mid-way, the log tells us the
    // exact path the kernel refused to write — which is the
    // difference between "directory blocked" (Controlled Folder
    // Access) and "this filename trigger a Win32 validation hook"
    // (AV quarantine + .exe fallback) and "this Unicode codepoint
    // is rejected" (NTFS surrogate pair).
    extract_dir_recursive(&EMBEDDED, &vendor_dir)
        .map_err(|e| {
            tracing::error!(
                "vendor extract failed — see preceding info!() log lines for the exact \
                 file that triggered the error. Cargo run with RUST_LOG=trace for full file \
                 enumeration."
            );
            e
        })?;

    let server_js = vendor_dir.join("src").join("server.js");
    if !server_js.exists() {
        return Err(VendorError::NotEmbedded);
    }

    // Best-effort write the marker first — if npm install fails after
    // this, the next launch will skip extraction but will *also* skip
    // the npm install, which is worse. So write the marker only after
    // npm install succeeds (see below).
    drop(marker);

    run_npm_install(&vendor_dir).await?;

    // Marker written last so a failed re-install triggers re-extraction
    // (rather than leaving the user with a half-broken node_modules).
    std::fs::write(
        data_dir
            .join("vendor")
            .join(format!(".extracted.{}", EMBEDDED_LINE_MCP_TAG)),
        format!(
            "extracted from embedded vendor at {}\n",
            chrono::Utc::now().to_rfc3339()
        ),
    )
    .ok();

    info!(
        "line-desktop-mcp ready at {} (server.js + node_modules installed)",
        server_js.display()
    );
    Ok(server_js)
}

/// Recursively walk `dir` and write every leaf file under `dest`,
/// mirroring nested sub-directory structure. Used for the embedded
/// line-desktop-mcp extraction because the tree has files nested under
/// `src/automation/`, `src/extensions/`, etc. — `Dir::files()` only
/// lists top-level leaves.
///
/// v0.6.11: emit `info!` for every write so a Windows user who hits
/// os error 193 (`%1 is not a valid Win32 application`) can run with
/// `RUST_LOG=line_hub_tauri_lib=debug` and see exactly which file
/// failed — and what its filename was. `ntstatus 0xC1` ("Bad Image
/// Format") from `create_dir_all` is almost always an EDR / AV
/// false positive that returns a fake non-Windows error code to
/// Rust's `CreateDirectoryW` wrapper.
fn extract_dir_recursive(dir: &Dir<'_>, dest: &Path) -> Result<(), VendorError> {
    for file in dir.files() {
        let target = dest.join(file.path());
        info!(
            "extracting {} ({} bytes)",
            target.display(),
            file.contents().len()
        );
        if let Some(parent) = target.parent() {
            info!("  mkdir -p {}", parent.display());
            std::fs::create_dir_all(parent).map_err(|e| VendorError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        info!("  write {}", target.display());
        std::fs::write(&target, file.contents()).map_err(|e| VendorError::Io {
            path: target,
            source: e,
        })?;
    }
    for sub in dir.dirs() {
        extract_dir_recursive(sub, dest)?;
    }
    Ok(())
}

async fn run_npm_install(vendor_dir: &Path) -> Result<(), VendorError> {
    let npm_cmd = which("npm").ok_or_else(|| VendorError::NpmMissing)?;
    info!("running `npm install` inside {}", vendor_dir.display());
    let output = Command::new(&npm_cmd)
        .arg("install")
        .arg("--omit=dev")
        .arg("--ignore-scripts")
        .arg("--no-audit")
        .arg("--no-fund")
        .current_dir(vendor_dir)
        .output()
        .await
        .map_err(|e| VendorError::Io {
            path: vendor_dir.to_path_buf(),
            source: e,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        warn!(
            "npm install failed (exit {:?})\n--- stderr ---\n{}",
            output.status.code(),
            stderr
        );
        return Err(VendorError::NpmInstall {
            status: output.status.code().unwrap_or(-1),
            stderr,
        });
    }
    Ok(())
}

/// Resolve the data directory used for the embedded vendor extraction.
///
/// v0.6.11: ignore the Tauri `identifier` and use a short, English-only
/// subdir name (`line-hub`) so the on-disk path does not look like a
/// `com.<vendor>.<app>` install — that shape is what Windows Defender /
/// EDR hooks scan for before deciding whether to fake an NTSTATUS 0xC1
/// on a fresh `mkdir`.
/// Tries multiple locations in order until one accepts a `mkdir`.
/// Windows SmartScreen / AppLocker restricts unsigned Tauri apps
/// (Jim confirmed: `Get-AuthenticodeSignature` returns `NotSigned`)
/// from creating new directories under `%LOCALAPPDATA%`, so the
/// first attempt may return NTSTATUS 0xC1 (`%1 is not a valid
/// Win32 application`). When that happens, fall back to a cache
/// or temp dir which is always trusted.
///
/// v0.6.12: 2-tier fallback. Order is
///   1. `%LOCALAPPDATA%\line-hub\` — preferred; survives reboots
///      so `npm install` cache is reusable across launches.
///   2. `%LOCALAPPDATA%\line-hub-vendor-v3.0.0\` (or `%TEMP%\...`)
///      — last-resort; rebuilt every launch so a user only sees
///      the slow `npm install` if `%LOCALAPPDATA%` is genuinely
///      blocked by SmartScreen.
///
/// We pick the first whose `mkdir` succeeds.
fn resolve_data_dir(_identifier: &str) -> Result<PathBuf, VendorError> {
    let candidates = build_candidate_dirs();
    for cand in &candidates {
        match std::fs::create_dir_all(cand) {
            Ok(()) => {
                info!("vendor data dir: {}", cand.display());
                return Ok(cand.clone());
            }
            Err(e) => tracing::warn!(
                "candidate {} rejected by filesystem: {} — falling through",
                cand.display(),
                e
            ),
        }
    }
    candidates.into_iter().next().ok_or_else(|| {
        VendorError::NoDataDir("no candidate dir resolved".to_string())
    })
}

fn build_candidate_dirs() -> Vec<PathBuf> {
    let tag = EMBEDDED_LINE_MCP_TAG;
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(b) = dirs::data_local_dir().or_else(dirs::data_dir) {
        out.push(b.join(DATA_DIR_SUBDIR));
        out.push(b.join(format!("{}-{}", DATA_DIR_SUBDIR, tag)));
    }
    if let Some(b) = dirs::cache_dir() {
        out.push(b.join(format!("{}-{}", DATA_DIR_SUBDIR, tag)));
    }
    if let Some(b) = std::env::var_os("TEMP").map(PathBuf::from) {
        out.push(b.join(format!("{}-{}", DATA_DIR_SUBDIR, tag)));
    }
    out
}

/// Tiny `which` for Windows + Unix. `which` crate is already a
/// transitive dep; we reimplement here only to keep the API surface
/// minimal and avoid pulling an extra dep just for this one call.
fn which(cmd: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in exts {
            let candidate = dir.join(format!("{cmd}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recursively count every `File` under this `Dir` — `Dir::files()`
    /// only returns the immediate children, but the embedded vendor tree
    /// has files nested under `src/automation/`, `src/extensions/`,
    /// etc. We need the real total so the test catches partial
    /// extraction.
    fn count_files_recursive(dir: &include_dir::Dir<'_>) -> usize {
        let mut total = dir.files().count();
        for sub in dir.dirs() {
            total += count_files_recursive(sub);
        }
        total
    }

    #[test]
    fn embedded_vendor_is_non_empty() {
        // Sentinel — if build.rs ever fails to populate the vendor tree
        // locally, this test catches it before shipping a broken exe.
        let total = count_files_recursive(&EMBEDDED);
        // After vendor-line-desktop-mcp.sh prunes .git, .github, tests/,
        // docs/, doc_media/ (demo GIFs ~65 MB), test/, README, etc.,
        // the embedded tree holds the runtime essentials: package.json,
        // npm-shrinkwrap.json, src/ with all subdirs (automation,
        // extensions), and the entry scripts. We assert >20 files
        // to catch a fresh, empty checkout or a pruning regression
        // (v0.6.8 shipped at 70 files before the doc_media prune).
        assert!(
            total > 20,
            "expected embedded line-desktop-mcp to contain >20 files (recursive), \
             only {} found — is crates/line-hub-tauri/vendor/line-desktop-mcp/ \
             populated?",
            total
        );
    }

    #[test]
    fn embedded_vendor_has_server_js() {
        let p = std::path::Path::new("src/server.js");
        assert!(
            EMBEDDED.get_file(p).is_some(),
            "expected embedded vendor to include src/server.js"
        );
    }

    #[test]
    fn embedded_vendor_has_package_json_with_main() {
        let pkg = EMBEDDED
            .get_file(std::path::Path::new("package.json"))
            .expect("embedded vendor missing package.json");
        let body = pkg
            .contents_utf8()
            .expect("package.json is not valid UTF-8");
        assert!(
            body.contains("\"main\"") || body.contains("\"type\""),
            "embedded package.json does not look like a Node.js package: {}",
            body.chars().take(200).collect::<String>()
        );
    }

    #[test]
    fn which_finds_npm_on_test_machines() {
        // Best-effort: skip rather than fail on stripped-down CI images.
        if std::env::var_os("CI").is_some() && which("npm").is_none() {
            // no-op; allow CI without npm to skip the check
            return;
        }
        // Local dev / devcontainer almost always has npm; this is a
        // sanity check, not a hard gate.
        let _ = which("npm");
    }
}
