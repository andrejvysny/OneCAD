# 0015 — The assistant host is a supervised Bun child process

**Status:** Accepted (2026-09-14)
**Context:** The AI modeling assistant program, work package WP-AI1.

## Context

OneCAD is adding a local AI assistant built on AgentKit, a TypeScript framework whose
durable store (`@agentkit/adapters-sqlite`) is Bun-only by construction — it imports
`bun:sqlite`, a runtime builtin. The orchestration loop, task runner, proposal lifecycle
and REST contract are all TypeScript.

Three placements were available: run AgentKit inside the webview, reimplement its
orchestration in Rust, or run it as a separate supervised process.

## Decision

Run the AgentKit host as a **separate child process**, compiled with `bun build --compile`
into a standalone executable, spawned and supervised by the Rust app, bundled through
Tauri's `externalBin`.

The webview does not run AgentKit. Rust does not reimplement it.

## Why not the webview

The host owns a SQLite database, a durable task queue with leases, and a recovery pass on
boot. None of that belongs in a renderer that a reload discards. A webview also cannot hold
the trust boundary: everything it can reach, page content can reach.

## Why not a Rust rewrite

It would be a second implementation of orchestration semantics — event sequencing, lease
fencing, proposal state machines — that AgentKit already has under test. The boundary worth
owning in Rust is the CAD one, not the chat loop. Rewriting would move effort away from the
part only OneCAD can do.

## Why a separate process, specifically

It gives a real failure boundary. The assistant can crash, wedge, or be restarted without
touching the geometry worker or the open document, and the supervisor can refuse to restart
a flapping child. The same argument already justifies the OCCT worker's process boundary.

## Consequences

**Accepted cost: ~99 MB.** Measured on this branch: a compiled hello-world is 99,295,580
bytes and the full assistant host is 99,399,737 — the Bun runtime is 99.3 MB of that and
all our code plus AgentKit adds 104 KB. Minifying changes nothing. This is the floor for
embedding any JavaScript runtime (Node's SEA is comparable), so it is not a cost that
better bundling recovers. Dropping AgentKit's MCP packages is still worth doing for attack
surface and startup, but it is not a size decision.

**Accepted cost: a second staged binary.** `tauri_build::build()` resolves every
`externalBin` entry in the build script, so a missing file fails `cargo check`, `clippy`
and `test` before a line compiles — measured, not predicted. Every developer clone and
every CI job that runs cargo must now stage two binaries. `scripts/build-assistant-host.sh`
exists to make the second one a seconds-long command rather than the worker's minutes.

**Naming.** This process is *not* called a sidecar in OneCAD prose. In this codebase
"sidecar" already means the C++ OCCT worker, and reusing it for a second, unrelated child
process would make every existing comment ambiguous.

The host is started lazily on first assistant use, not at app boot, and its restart is
decoupled from the geometry worker's.
