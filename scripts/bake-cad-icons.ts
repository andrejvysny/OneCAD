// Bakes the colour-isometric icon set (docs/design/native-ui/final/cad-icons.js)
// into static SVG masters for the native UI. The design renderer builds every
// icon from a small 3D description at runtime; running it once here means no
// JavaScript or projection code ships in the app.
//
// Output: crates/onecad-widgets/assets/icons/{light,dark}.json
//   { "sizes": [14, ...], "gap": "#fe01fe", "icons": { "<name>": { "<size>": "<svg>" } } }
// `currentColor` stays in the SVG (the host tints it); the `gap` sentinel marks
// knock-out strokes that the host replaces with the surface colour behind the icon.
//
// Usage: bun scripts/bake-cad-icons.ts
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { runInNewContext } from "node:vm";

const root = join(dirname(new URL(import.meta.url).pathname), "..");
const source = join(root, "docs/design/native-ui/final/cad-icons.js");
const outDir = join(root, "crates/onecad-widgets/assets/icons");

const SIZES = [14, 16, 18, 20, 24, 32];
const THEMES = ["light", "dark"] as const;
const GAP = "#fe01fe";

type CadIcons = { names: string[]; svg: (name: string, theme: string, size: number, bg: string) => string };

function loadRenderer(): CadIcons {
  const window: { CadIcons?: CadIcons } = {};
  // Only `window` and a registry stub are touched at load time; the custom
  // element is never instantiated here.
  const customElements = { get: () => true, define: () => undefined };
  runInNewContext(readFileSync(source, "utf8"), { window, customElements, Math });
  if (!window.CadIcons) throw new Error("cad-icons.js did not register window.CadIcons");
  return window.CadIcons;
}

// Sidebar toggle glyphs, taken verbatim from the inline SVG in OneCAD Final.dc.html.
// `*Open` adds the tinted panel the design shows while that panel is visible.
function panelGlyph(side: "left" | "right", open: boolean, size: number): string {
  const x = side === "left" ? 9.5 : 14.5;
  const fillX = side === "left" ? 4.2 : 15.5;
  const fill = open
    ? `<rect x="${fillX}" y="5.7" width="4.3" height="12.6" rx="1.3" fill="currentColor" fill-opacity=".3" stroke="none"/>`
    : "";
  return (
    `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" ` +
    `stroke-width="1.7" stroke-linejoin="round" stroke-linecap="round">` +
    `<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M${x} 4.5v15"/>${fill}</svg>`
  );
}

const EXTRA: Record<string, (size: number) => string> = {
  panelLeft: (s) => panelGlyph("left", false, s),
  panelLeftOpen: (s) => panelGlyph("left", true, s),
  panelRight: (s) => panelGlyph("right", false, s),
  panelRightOpen: (s) => panelGlyph("right", true, s),
};

const icons = loadRenderer();
const names = [...icons.names].sort();
mkdirSync(outDir, { recursive: true });
for (const theme of THEMES) {
  const out: Record<string, Record<string, string>> = {};
  for (const name of names) {
    out[name] = Object.fromEntries(SIZES.map((s) => [String(s), icons.svg(name, theme, s, GAP)]));
  }
  for (const [name, draw] of Object.entries(EXTRA)) {
    if (out[name]) throw new Error(`extra glyph ${name} collides with the icon set`);
    out[name] = Object.fromEntries(SIZES.map((s) => [String(s), draw(s)]));
  }
  const sorted = Object.fromEntries(Object.keys(out).sort().map((k) => [k, out[k]]));
  writeFileSync(join(outDir, `${theme}.json`), JSON.stringify({ sizes: SIZES, gap: GAP, icons: sorted }) + "\n");
}
console.log(`baked ${names.length + Object.keys(EXTRA).length} icons × ${SIZES.length} sizes × ${THEMES.length} themes`);
