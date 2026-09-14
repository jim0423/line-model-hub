#!/usr/bin/env bash
# scripts/vendor-line-desktop-mcp.sh
#
# Vendors a pinned copy of line-desktop-mcp into vendor/line-desktop-mcp/
# for bundling into the Tauri installer. Called from `beforeBuildCommand`
# in tauri.conf.json — runs both locally and in CI before `cargo tauri
# build`.
#
# Why this exists: v0.6.1 ships line-desktop-mcp bundled so end users do
# not need to `git clone` + `npm install` separately. We clone once at
# build time, check in a SHA-pinned tag, and let NSIS post-install do the
# user-side `npm install --omit=dev --ignore-scripts`.
#
# Idempotent: skip the clone if vendor/line-desktop-mcp/package.json is
# already present and matches the pinned version.
#
# Outputs:
#   vendor/line-desktop-mcp/src/server.js   — entry script (bundled)
#   vendor/line-desktop-mcp/package.json    — npm manifest (bundled)
#   vendor/line-desktop-mcp/package-lock.json — npm lock (bundled, if present)
#
# Size: about 5 MB without node_modules (bundled), another ~50 MB if
# build-time `npm install` runs in CI for smoke test.

set -euo pipefail

# Pin a specific tag. v3.0.0 ships the 29-tool surface v0.5.0+. Bumping
# this requires a fresh build + installer.
LINE_DESKTOP_MCP_TAG="${LINE_DESKTOP_MCP_TAG:-v3.0.0}"
REPO="https://github.com/bensonmaxai/line-desktop-mcp.git"
DEST="vendor/line-desktop-mcp"

# Marker file — if the destination is already populated and matches the
# pinned tag, skip the clone. This keeps local rebuilds fast.
MARKER="${DEST}/.vendor.tag"

if [[ -f "${DEST}/package.json" && -f "${MARKER}" ]]; then
    PINNED=$(cat "${MARKER}")
    if [[ "${PINNED}" == "${LINE_DESKTOP_MCP_TAG}" ]]; then
        echo "[vendor] line-desktop-mcp ${LINE_DESKTOP_MCP_TAG} already present — skipping clone"
        exit 0
    fi
    echo "[vendor] pinned tag changed (${PINNED} -> ${LINE_DESKTOP_MCP_TAG}) — re-cloning"
    rm -rf "${DEST}"
fi

mkdir -p vendor
echo "[vendor] cloning ${REPO} @ ${LINE_DESKTOP_MCP_TAG} (shallow, --depth 1)..."
git clone --depth 1 --branch "${LINE_DESKTOP_MCP_TAG}" "${REPO}" "${DEST}.tmp"
rm -rf "${DEST}"
mv "${DEST}.tmp" "${DEST}"

# Drop anything we do not need inside the installer — git history, dev
# configs, GH Actions. Stays under ~5 MB.
rm -rf "${DEST}/.git"
rm -rf "${DEST}/.github"
rm -rf "${DEST}/tests" 2>/dev/null || true
rm -rf "${DEST}/docs"   2>/dev/null || true
rm -f  "${DEST}/.gitignore"
rm -f  "${DEST}/.npmignore" 2>/dev/null || true
rm -f  "${DEST}/README.md" 2>/dev/null || true

# Sanity check: confirm the entry script exists where the Rust side
# expects it. If the upstream layout shifts, fail loudly.
if [[ ! -f "${DEST}/src/server.js" ]]; then
    echo "[vendor] FATAL: ${DEST}/src/server.js missing after clone — upstream layout changed?" >&2
    exit 1
fi

echo "${LINE_DESKTOP_MCP_TAG}" > "${MARKER}"

# Optional smoke check: if npm is on PATH, run a fast install to verify
# the lockfile resolves before we ship it. Skipped when SKIP_NPM_INSTALL
# is set (CI runner without Node, dev sandbox without internet, etc.).
if [[ -z "${SKIP_NPM_INSTALL:-}" ]] && command -v npm >/dev/null 2>&1; then
    echo "[vendor] smoke-testing npm install (skip postinstall)..."
    (cd "${DEST}" && npm install --omit=dev --ignore-scripts --no-audit --no-fund) \
        || { echo "[vendor] FATAL: npm install smoke test failed" >&2; exit 1; }
    # Clean node_modules again — installer will run npm install per-user.
    rm -rf "${DEST}/node_modules"
    echo "[vendor] smoke test passed, removed node_modules"
fi

echo "[vendor] done — ${DEST}/src/server.js ready for bundling"
