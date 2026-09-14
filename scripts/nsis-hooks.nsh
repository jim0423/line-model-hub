; scripts/nsis-hooks.nsh
;
; Custom NSIS hooks for the Line 小幫手 installer. Runs once after
; install (or uninstall) and is responsible for bootstrapping the
; bundled line-desktop-mcp Node server:
;
;   - On install: cd to $INSTDIR\resources\line-desktop-mcp, run
;     `npm install --omit=dev --ignore-scripts`, write a `.installed`
;     sentinel so subsequent launches can skip the check.
;   - On uninstall: remove the sentinel + node_modules (everything else
;     falls away with $INSTDIR).
;
; The hook runs *after* files are extracted. If node.exe is missing we
; surface a clear message in the installer finish page — the rest of the
; app still works (chat is read-only without MCP), but Spawn LINE MCP
; will fail until the user installs Node.js.

!macro customInstall
    ; $INSTDIR is set by Tauri to the chosen install location, e.g.
    ; C:\Program Files\Line 小幫手.
    SetRegView 64
    WriteRegDWORD HKLM "Software\LineBangshou\Install" "NodeMcpBootstrapAttempted" 1

    ; ---- Detect node.exe ----
    nsExec::ExecToLog 'cmd /c where node > "%TEMP%\lh_node_path.txt" 2>&1'
    Pop $0
    ${If} $0 != 0
        ; Where failed (no node on PATH). Log + emit the message the
        ; installer finish page will pick up via the registry.
        FileOpen $4 "$INSTDIR\install-node-missing.txt" w
        FileWrite $4 "Node.js (node.exe) was not found on this machine's PATH.$\r$\n"
        FileWrite $4 "line-desktop-mcp will not start until Node.js is installed.$\r$\n$\r$\n"
        FileWrite $4 "Install Node.js 22 LTS from https://nodejs.org/ and re-run this installer (or just restart Line \x5Cxe5\x5Cxb0\x5Cxa1\x5Cxeb\x5Cxe6\x8E\x5Cx92\x5Cxe6\x89\x8B — the bootstrap will retry on launch).$\r$\n"
        FileClose $4
        WriteRegDWORD HKLM "Software\LineBangshou\Install" "NodeMcpBootstrapped" 0
        Goto install_done
    ${EndIf}

    ; ---- Run npm install inside the bundled line-desktop-mcp ----
    SetOutPath "$INSTDIR\resources\line-desktop-mcp"
    nsExec::ExecToLog 'cmd /c cd /d "$INSTDIR\resources\line-desktop-mcp" && npm install --omit=dev --ignore-scripts --no-audit --no-fund > "$TEMP\line-desktop-mcp-install.log" 2>&1'
    Pop $0
    ${If} $0 == 0
        ; Sentinel: Rust side checks for this to skip re-bootstrap.
        FileOpen $4 "$INSTDIR\resources\line-desktop-mcp\.installed" w
        FileWrite $4 "installed at install time, ok$\r$\n"
        FileClose $4
        WriteRegDWORD HKLM "Software\LineBangshou\Install" "NodeMcpBootstrapped" 1
    ${Else}
        ; npm install failed (no network, lockfile drift, etc.) — the
        ; app still launches but Spawn LINE MCP will retry the
        ; bootstrap on next launch (see default_entry_path in mcp.rs).
        WriteRegDWORD HKLM "Software\LineBangshou\Install" "NodeMcpBootstrapped" 0
        FileOpen $4 "$INSTDIR\install-npm-failed.txt" w
        FileWrite $4 "npm install for line-desktop-mcp failed during installation.$\r$\n"
        FileWrite $4 "The app will retry the bootstrap the first time you click 'Spawn LINE MCP'.$\r$\n"
        FileClose $4
    ${EndIf}

    install_done:
!macroend

!macro customUnInstall
    ; Keep $INSTDIR intact during uninstall so we can clean up our
    ; generated files (npm cache + sentinels) before the rest gets
    ; wiped by Tauri's default uninstaller.
    ${If} ${FileExists} "$INSTDIR\resources\line-desktop-mcp\.installed"
        Delete "$INSTDIR\resources\line-desktop-mcp\.installed"
    ${EndIf}
    ${If} ${FileExists} "$TEMP\line-desktop-mcp-install.log"
        Delete "$TEMP\line-desktop-mcp-install.log"
    ${EndIf}
    DeleteRegKey HKLM "Software\LineBangshou"
!macroend
