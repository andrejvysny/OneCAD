/*
 * Injects the pre-paint splash <style> into index.html at transformIndexHtml
 * time (runs in both `vite dev` and `vite build`). tokens.css is the SOLE
 * source of design colors (grep-gated, no raw hex outside it) — this reads
 * --color-canvas/--color-ink out of BOTH theme blocks so the splash never
 * hardcodes a color of its own and can never drift from the design tokens.
 * A token rename that breaks extraction throws at build time: shipping the
 * wrong background silently is worse than failing loudly.
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";

const TOKENS_PATH = resolve(dirname(fileURLToPath(import.meta.url)), "../src/styles/tokens.css");

function extractBlock(css: string, blockRe: RegExp, label: string): string {
  const match = css.match(blockRe);
  if (!match) throw new Error(`vite-splash-plugin: could not find ${label} block in tokens.css`);
  return match[1];
}

function extractVar(block: string, name: string, label: string): string {
  const match = block.match(new RegExp(`${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`vite-splash-plugin: ${name} not found in ${label} block`);
  return match[1].trim();
}

export default function splashPlugin(): Plugin {
  return {
    name: "onecad-splash",
    transformIndexHtml() {
      const css = readFileSync(TOKENS_PATH, "utf-8");
      const light = extractBlock(css, /@theme\s*{([\s\S]*?)\n}/, "light (@theme)");
      const dark = extractBlock(
        css,
        /:root\[data-theme=["']dark["']\]\s*{([\s\S]*?)\n}/,
        'dark (:root[data-theme="dark"])',
      );
      const lightBg = extractVar(light, "--color-canvas", "light");
      const lightFg = extractVar(light, "--color-ink", "light");
      const darkBg = extractVar(dark, "--color-canvas", "dark");
      const darkFg = extractVar(dark, "--color-ink", "dark");

      const style = `
:root{--splash-bg:${lightBg};--splash-fg:${lightFg}}
:root[data-theme="dark"]{--splash-bg:${darkBg};--splash-fg:${darkFg}}
html{background-color:var(--splash-bg)}
#splash{position:fixed;inset:0;pointer-events:none;display:flex;align-items:center;justify-content:center;gap:.5rem;color:var(--splash-fg);font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;font-size:1.125rem;letter-spacing:.02em}
#splash .dot{width:6px;height:6px;border-radius:50%;background:currentColor;animation:splash-pulse 1.2s ease-in-out infinite}
@keyframes splash-pulse{0%,100%{opacity:.3}50%{opacity:1}}
#root:not(:empty)+#splash{display:none}
`.trim();

      return [{ tag: "style", children: style, injectTo: "head" }];
    },
  };
}
