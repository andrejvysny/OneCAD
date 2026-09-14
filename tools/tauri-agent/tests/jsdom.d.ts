/**
 * Minimal ambient types for jsdom.
 *
 * jsdom is already present (vitest pulls it in) but `@types/jsdom` is not, and a
 * test-only shim is cheaper than adding a dependency. Declares only the surface
 * `snapshotScript.test.ts` touches.
 */
declare module "jsdom" {
  export interface JSDOMOptions {
    pretendToBeVisual?: boolean;
    url?: string;
  }
  export class JSDOM {
    constructor(html?: string, options?: JSDOMOptions);
    readonly window: Window & typeof globalThis;
  }
}
