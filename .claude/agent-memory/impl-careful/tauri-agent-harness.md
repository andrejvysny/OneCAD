---
name: tauri-agent-harness
description: Gotchas for tools/tauri-agent (the MCP real-user harness) — SDK 1.30 registerTool blows TS instantiation depth, bun-types needs @types/bun at root, rm is denied so tests cannot shell out to clean temp dirs
metadata:
  type: project
---

`tools/tauri-agent/` is a bun-executed TypeScript MCP server; it has no deps of its own and resolves
everything from the repo-root `node_modules`. Its `package.json` deliberately has no `dependencies`.

- **`server.registerTool(name, {inputSchema: <zod v3 raw shape>}, cb)` from `@modelcontextprotocol/sdk`
  1.30 fails with TS2589 "Type instantiation is excessively deep"** for schemas of realistic size —
  the SDK re-infers each shape through `ShapeOutput`. Fix used in `src/mcp/defineTool.ts`: bind the
  method and cast it to a narrow non-generic `RawRegister` type. Runtime behaviour is unchanged.
  **Why:** the generic blowup is in the SDK's types, not in our schemas, so it cannot be fixed locally.
  **How to apply:** any new call site that passes a zod shape to the SDK should go through
  `defineTool`, not `registerTool` directly.
- **`bun-types` is only resolvable because `@types/bun` is a root devDependency** (added 2026-09-13).
  `tsconfig.json` in `tools/tauri-agent` uses `"types": ["bun-types"]` per CONTRACTS.md; `bun-types`
  arrives transitively from `@types/bun`. Removing `@types/bun` silently breaks `import.meta.dir` and
  `Bun.spawn` typings there.
- **`rm` is denied by `.claude/settings.local.json`**, so a bash one-liner that creates and cleans a
  temp dir is refused outright. Use the scratchpad dir, or clean up from inside the test with
  `node:fs` `rmSync` (which is what `tests/*.test.ts` do).
- The MCP stdio transport frames messages as **newline-delimited JSON with no Content-Length header**;
  `tests/server.smoke.test.ts` drives the real server that way (initialize → notifications/initialized
  → tools/list → tools/call) and finishes in ~60 ms.
