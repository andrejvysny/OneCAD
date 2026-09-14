# OneCAD logs for the harness (source: `docs/DEBUGGING.md`)

The launched dev app writes `ONECAD_LOG_DIR=<artifacts>/<session>/app-logs/dev.jsonl` (the harness
sets it; a hand-started app writes `logs/dev.jsonl` at the repo root, truncated on every start —
first line is `session.start`). `observe_logs` tails it from a cursor.

Lanes (`target` field): Rust module paths (`onecad_lib::…`), `worker` (forwarded C++ stderr,
`epoch` field, no span), `fe` (webview events: `tag`, `seq`, `feTs`, `ctx`), `onecad_protocol::frames`
(per OCW1 frame, debug-gated), `panic`. `RUST_LOG` default already includes `fe=debug,worker=debug`;
a bare `info` would silently drop both.

FE tags worth grepping: `ipc` (every invoke round trip: `{cmd, durMs}`), `fsm` (model-tool phase
transitions), `hint` (every status-bar hint the user saw), `err` (uncaught FE errors), `sketch`,
`mesh`, `repair`, `worker` (FE mirror of worker state).

First-look queries with `observe_logs`:
- `{level:"error"}` — anything red.
- `{grep:"regen:"}` — regen outcome and failed-step lines.
- `{lane:"fe", grep:"\"tag\":\"hint\""}` — what the user was told.
- `{lane:"fe", grep:"save_document"}` / `{grep:"apply_operation"}` — IPC evidence for a step.
- `{lane:"worker"}` — kernel side.

DEV-only white-box surfaces (present in `launch:"dev"`, absent in the bundled artifact):
`window.__logsDump()`, `window.__stores` (13 keys incl. `document`, `sketch`, `viewport`, `tool`),
`window.__client` (`getOperationParams`), `?vpdebug` → `window.__vpEngine` (not set in the dev URL
the harness opens). Use them only through `debug_eval_js` and label the result diagnostic.
