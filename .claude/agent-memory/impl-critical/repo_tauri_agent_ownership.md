---
name: repo-tauri-agent-ownership
description: Facts behind tauri-agent process ownership and the agent_identity handshake — where the config root is, how Tauri phrases an unknown command, AppHandle::config is inherent, DELETE /session does not exit the app
metadata:
  type: project
---

Facts that bit while making `tools/tauri-agent` refuse to kill an app it did not launch (WP-A).

- **`tauri-agent.config.json` lives at the REPO ROOT**, not in `tools/tauri-agent/`. So
  `ResolvedConfig.root` is the repo root and the app's compile-time `CARGO_MANIFEST_DIR`
  (`<root>/src-tauri`) resolves INSIDE it. The `agent_identity` gate compares exactly those
  two; moving the config file down a level would hard-fail every session with
  `APP_IDENTITY_MISMATCH`.
- **Tauri 2.11.5 rejects an unregistered command with `"Command <name> not found"`**
  (`src/webview/mod.rs:1905`) or `"<cmd> not allowed. Command not found"`
  (`src/ipc/authority.rs:404`). That exact phrasing is what separates "this app predates the
  handshake" (a warning) from "this is a different app" (a refusal), so a rewording upstream
  would silently turn old-binary attaches into hard failures.
- **`AppHandle::config()` is INHERENT in tauri 2.11** — importing `tauri::Manager` for it is an
  `unused_imports` error under `-D warnings`.
- **`DELETE /session` on `tauri-plugin-wdio-webdriver` 1.3 only removes the session from a map**
  (`src/server/handlers/session.rs:205` → `webdriver/session.rs:131`); it never exits the app.
  So dropping the bridge is safe on a session that ATTACHED — the only thing that can end
  such an app is a signal, which is now gated on `ProcessOwnership`.
- **`ps -o comm=` is the executable path; `ps -o command=` is the full command line.** Every
  ownership decision uses `comm` (the argument list is not evidence of what is running, and
  splitting a command line on whitespace mis-derives argv[0] for a path containing a space);
  the command line is kept for reports only.

**Why:** each one is invisible from the type signatures, and the first two decide between
"the agent refuses to drive a stranger" and "the agent bricks itself".
**How to apply:** when touching `tools/tauri-agent/src/session/**` or
`src-tauri/src/tauri_e2e.rs`.
