/*
 * Production build-time replacement for `./mockClient` (see the eviction
 * plugin in vite.config.ts). The real mockClient.ts drags in three.js's
 * ShapeUtils/Vector2 path (via mockRegions) plus a synthetic document model
 * this build never runs; this throws instead of shipping any of it.
 *
 * `mockClient` is the only export needed here: client.ts's `createClient()`
 * must stay sync (no dynamic import), so `./mockClient` is always statically
 * imported and Rollup resolves that specifier against this stub in prod. The
 * demo-only named exports (`emitMockDocumentChanged`, `setMockLatency`, etc.)
 * used to need throwing stand-ins too, because ViewportRoot.tsx imported them
 * statically for its `?vpdemo`/`?sketchdemo`/`?toolsdemo` blocks — that import
 * is now dynamic and DEV-gated (see viewport/devDemos.ts), so those names are
 * never statically imported against this specifier anymore and don't need a
 * stand-in.
 */
const unavailable = (): never => {
  throw new Error("mock client unavailable in production build");
};

export const mockClient = new Proxy(
  {},
  { get: unavailable },
) as unknown as typeof import("./mockClient").mockClient;
