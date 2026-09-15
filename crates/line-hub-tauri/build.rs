//! v0.6.8: tauri_build runs as usual, plus we bake the vendored
//! line-desktop-mcp directory into the binary as a `static Vendor`
//! constant so the runtime can unpack it to `~/.line-hub/vendor/...` on
//! first launch.
//!
//! Why this exists: in v0.6.1..v0.6.7 we relied on
//! `tauri.conf.json::bundle.resources` to ship line-desktop-mcp inside
//! the NSIS installer, but the NSIS bundler silently skipped every
//! resource whose `cwd.join()` resolution did not exactly match the
//! on-disk layout. The result was an installer that `cargo tauri build`
//! accepted as success yet contained zero references to
//! line-desktop-mcp / server.js / package.json — verified in CI by the
//! `Verify bundled resources landed in installer` step
//! (run 34828069067, log line 1465). Embedding the sources directly in
//! the exe sidesteps the bundler entirely.

use include_dir::{include_dir, Dir};
use std::path::PathBuf;

const VENDOR_DIR_NAME: &str = "line-desktop-mcp";

// `include_dir!` requires the path to exist at parse time. The CI
// `beforeBuildCommand` (`scripts/vendor-line-desktop-mcp.sh`) guarantees
// `crates/line-hub-tauri/vendor/line-desktop-mcp/` is populated before
// `cargo tauri build` runs, so this resolves to the freshly-cloned
// v3.0.0 tag.
static VENDOR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/vendor/line-desktop-mcp");

fn main() {
    tauri_build::build();

    // Surface a clear compile-time error if the vendored tree is empty
    // (e.g. someone forgot to run the vendor script, or the upstream
    // layout shifted). Better than a confusing runtime "spawn failed"
    // much later.
    let src_path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "vendor", VENDOR_DIR_NAME]
        .iter()
        .collect();
    println!("cargo:rerun-if-changed={}", src_path.display());
    if !src_path.join("package.json").exists() {
        panic!(
            "vendored line-desktop-mcp missing at `{}` — \
             run `bash scripts/vendor-line-desktop-mcp.sh` before \
             `cargo tauri build`.",
            src_path.display()
        );
    }
    // Re-run build.rs if any vendored file changes so the embedded bytes
    // get refreshed without a manual `cargo clean`.
    println!(
        "cargo:rerun-if-changed={}",
        src_path.join("src").display()
    );

    eprintln!(
        "[line-hub-tauri build.rs] embedded line-desktop-mcp v? — \
         {} files, {} bytes (uncompressed)",
        VENDOR.files().count(),
        VENDOR
            .files()
            .map(|f| f.contents().len())
            .sum::<usize>()
    );
}
